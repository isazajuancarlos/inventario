//! El panel, tejido EN EL SERVIDOR con `maud` (directiva 26).
//!
//! Sustituye al SPA de Angular 22 + Material que vivía en `frontend/` y viajaba
//! empotrado con `rust-embed` desde `estaticos/`. Lo que compra el cambio, y es
//! la razón de fondo: **el build deja de necesitar Node**. El HTML lo compila
//! `rustc` con los tipos del dominio dentro, así que una plantilla rota es un
//! error de compilación y no una pantalla en blanco a las tres semanas.
//!
//! `/api` (JSON) NO cambia: sigue siendo la superficie de integración, y este
//! panel es un segundo consumidor, server-rendered.
//!
//! htmx y `boi` van **vendorizados y embebidos** con `include_str!`, servidos
//! locales. Nunca por CDN: esto corre en una envasadora de agua de Medellín que
//! puede quedarse sin internet, y lo que no esté dentro del binario no existe.

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use maud::{DOCTYPE, Markup, html};
use serde::Deserialize;

use crate::rutas::{Estado, sesion_de};
use inventario_datos::{articulo, catalogo, usuario};

/// **Los datos, o la pantalla que dice que no se pudieron leer.**
///
/// Sustituye a `unwrap_or_default()`, que hacía indistinguibles «no hay
/// artículos» y «la consulta reventó» (directiva 30): la lista vacía se dibujaba
/// igual en los dos casos, con su «No hay artículos que encajen» tranquilizador,
/// y una base corrupta o un esquema sin migrar se leían como un inventario
/// vacío. `cartera_pagina` y `alistamiento_pagina` ya avisaban; el resto no, y
/// esa inconsistencia era el defecto.
///
/// Va como macro y no como función porque tiene que RETORNAR desde el llamante:
/// una función solo podría devolver un valor por omisión, que es justo lo que se
/// está quitando.
macro_rules! datos_o_aviso {
    ($empresa:expr, $que:expr, $consulta:expr) => {
        match $consulta {
            Ok(v) => v,
            Err(e) => {
                return aviso(
                    $empresa,
                    "No se pudo leer la base",
                    &format!("{}: {}", $que, mostrable(&e)),
                );
            }
        }
    };
}


// ── Lo vendorizado ────────────────────────────────────────────────────────────
// La MISMA versión de htmx y de `boi` que nexus y chasqui (directiva 26). Se
// comprueba con `md5sum`, no de memoria.
const HTMX_JS: &str = include_str!("../estaticos/htmx.min.js");
const BOI_JS: &str = include_str!("../estaticos/boi.js");
const PANEL_CSS: &str = include_str!("../estaticos/panel.css");
// La mitad estética de boi (tokens + tema + IBM Plex empotrada). md5
// 0525cc0173d3a098cb4465380aad00cd, coincide con el repo boi (directiva 9/26).
const BOI_CSS: &str = include_str!("../estaticos/boi.css");
const SUGERIR_JS: &str = include_str!("../estaticos/comportamientos/sugerir-de-opcion.js");

fn recurso(cuerpo: &'static str, tipo: &'static str) -> Response {
    ([(header::CONTENT_TYPE, tipo)], cuerpo).into_response()
}

pub async fn htmx_js() -> Response {
    recurso(HTMX_JS, "application/javascript; charset=utf-8")
}
pub async fn sugerir_js() -> Response {
    recurso(SUGERIR_JS, "application/javascript; charset=utf-8")
}

pub async fn boi_js() -> Response {
    recurso(BOI_JS, "application/javascript; charset=utf-8")
}
pub async fn panel_css() -> Response {
    recurso(PANEL_CSS, "text/css; charset=utf-8")
}
pub async fn boi_css() -> Response {
    recurso(BOI_CSS, "text/css; charset=utf-8")
}

// ── La envoltura ──────────────────────────────────────────────────────────────

/// Las secciones de la barra, en el orden en que se usan durante el día. El
/// alistamiento va aparte, al final: se mira el primer día y cuando algo no
/// deja trabajar, no todos los días.
const SECCIONES: [(&str, &str); 8] = [
    ("/articulos", "Artículos"),
    ("/movimientos", "Inventario"),
    ("/terceros", "Terceros"),
    ("/facturar", "Facturar"),
    ("/recaudar", "Recaudar"),
    ("/cartera", "Cartera"),
    ("/informes", "Informes"),
    ("/configuracion", "Configuración"),
];

/// El caparazón de toda pantalla con sesión. `activa` es la ruta actual, para
/// marcar la sección donde uno está — un menú que no dice dónde estás obliga a
/// mirar la URL.
pub fn envoltura(titulo: &str, empresa: &str, activa: Option<&str>, cuerpo: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="es" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (titulo) " · " (empresa) }
                link rel="stylesheet" href="/boi.css";
                link rel="stylesheet" href="/panel.css";
                // htmx como script clásico; boi como módulo. Los dos LOCALES: un `<script src>` a un CDN es justo
                // el vector que este stack existe para cerrar.
                script src="/htmx.min.js" {}
                script type="module" src="/boi.js" {}
                // El primer comportamiento, y nació como manda la directiva 26:
                // porque una pantalla lo pidió de verdad —el `data-precio` de
                // las líneas de factura, que era un atributo muerto—, no por
                // especulación. Se sirve bajo `/comportamientos/` para que su
                // `../boi.js` resuelva sin editarle el import, igual que en
                // chasqui, informes y medico.
                script type="module" src="/comportamientos/sugerir-de-opcion.js" {}
            }
            body {
                @if let Some(activa) = activa {
                    nav.barra {
                        span.empresa { (empresa) }
                        span.espacio {}
                        @for (ruta, nombre) in SECCIONES {
                            a href=(ruta) class=(if ruta == activa { "activo" } else { "" }) { (nombre) }
                        }
                        a.alistamiento href="/alistamiento"
                          title="Alistamiento: qué falta para operar" { "Alistamiento" }
                        form.salir method="post" action="/salir" {
                            button type="submit" { "Salir" }
                        }
                    }
                }
                main.contenido { (cuerpo) }
            }
        }
    }
}

// ── Entrada ───────────────────────────────────────────────────────────────────

/// El nombre de la empresa, con su defecto explícito. Antes de saberlo NO se
/// pone uno inventado: se deja el genérico, que no afirma nada sobre el negocio
/// del cliente (directiva 20).
async fn nombre_empresa(st: &Estado) -> String {
    catalogo::empresa_publica(&st.pool)
        .await
        .ok()
        .flatten()
        .map(|e| e.nombre)
        .unwrap_or_else(|| "Inventario".to_string())
}

pub async fn entrar_pagina(State(st): State<Estado>) -> Markup {
    let empresa = nombre_empresa(&st).await;
    // El cartel de la demostración solo aparece si hay credenciales de demo.
    // En una instalación de cliente no hay ninguna y aquí no se pinta nada:
    // nadie ve un usuario y una clave que no son suyos.
    let demo = crate::demo::credenciales();
    pagina_de_entrada(&empresa, demo, None)
}

fn pagina_de_entrada(empresa: &str, demo: Option<(String, String)>, error: Option<&str>) -> Markup {
    envoltura("Entrar", empresa, None, html! {
        div.centro {
            div.caja {
                h1 { (empresa) }
                @if let Some(e) = error {
                    div.error role="alert" { (e) }
                }
                @if let Some((usuario, clave)) = &demo {
                    div.demo {
                        b { "Demostración pública" }
                        // Los campos se rellenan solos: pedirle a quien llega
                        // que copie a mano un usuario y una clave que están tres
                        // centímetros más arriba es una barrera sin motivo.
                        p { "Entre con " code { (usuario) } " / " code { (clave) } }
                    }
                }
                form method="post" action="/entrar" {
                    label for="codigo" { "Usuario" }
                    input #codigo name="codigo" autocomplete="username" required
                          value=(demo.as_ref().map(|d| d.0.as_str()).unwrap_or(""));
                    label for="clave" { "Clave" }
                    input #clave type="password" name="clave" autocomplete="current-password"
                          required value=(demo.as_ref().map(|d| d.1.as_str()).unwrap_or(""));
                    button.primario type="submit" { "Entrar" }
                }
            }
        }
    })
}

#[derive(Deserialize)]
pub struct Credenciales {
    codigo: String,
    clave: String,
}

/// `POST /entrar` — el mismo flujo que `/api/sesion`, pero para el navegador:
/// en éxito, redirección con la cookie puesta; en fallo, la misma página con el
/// mensaje. No hay JSON aquí: lo consume una persona, no una máquina.
pub async fn entrar_enviar(
    State(st): State<Estado>,
    axum::Form(c): axum::Form<Credenciales>,
) -> Response {
    let empresa = nombre_empresa(&st).await;

    // El mismo mensaje tanto si el usuario no existe como si la clave está mal:
    // distinguirlos convierte la pantalla de entrada en un comprobador de quién
    // tiene cuenta.
    let malo = |empresa: &str| {
        (
            StatusCode::UNAUTHORIZED,
            pagina_de_entrada(empresa, crate::demo::credenciales(), Some("Usuario o clave incorrectos")),
        )
            .into_response()
    };

    let encontrado = match usuario::por_codigo(&st.pool, &c.codigo).await {
        Ok(v) => v,
        // Una base caída NO es «clave incorrecta». Devolver 401 aquí disfraza un
        // fallo real de operación normal y lo hace invisible en los registros
        // (directiva 20). `/api/sesion` sí los distingue; el panel lo había
        // degradado.
        Err(e) => return aviso(&empresa, "Entrar", &mostrable(&e)),
    };
    let Some((u, hash)) = encontrado else {
        return malo(&empresa);
    };
    if !crate::claves::verificar(&c.clave, &hash) {
        return malo(&empresa);
    }

    let vence = crate::rutas::ahora() + st.horas_sesion * 3600;
    let cookie = crate::sesion::emitir(&st.clave_firma, &crate::sesion::Sesion {
        usuario_id: u.id,
        rol: u.rol,
        vence,
    });
    let mut h = HeaderMap::new();
    h.insert(
        header::SET_COOKIE,
        crate::sesion::cabecera(&cookie, st.horas_sesion * 3600, st.cookie_segura)
            .parse()
            .expect("cabecera válida"),
    );
    h.insert(header::LOCATION, "/articulos".parse().expect("ruta válida"));
    (StatusCode::SEE_OTHER, h).into_response()
}

pub async fn salir_enviar(State(st): State<Estado>) -> Response {
    let mut h = HeaderMap::new();
    h.insert(
        header::SET_COOKIE,
        crate::sesion::cabecera_de_salida(st.cookie_segura)
            .parse()
            .expect("cabecera válida"),
    );
    h.insert(header::LOCATION, "/entrar".parse().expect("ruta válida"));
    (StatusCode::SEE_OTHER, h).into_response()
}

/// **Sesión Y PERMISO.** El permiso va como parámetro OBLIGATORIO, no opcional,
/// y eso es la corrección de un defecto real que introdujo la primera versión de
/// este panel: comprobaba solo la sesión, así que un usuario con rol `consulta`
/// —el del contador o el temporal que solo mira— podía registrar movimientos,
/// **emitir facturas contra una resolución de la DIAN** y recaudar por el panel,
/// cuando por `/api` recibía 403 en las tres. Un número de resolución quemado no
/// se recupera.
///
/// La forma importa tanto como el arreglo (directiva 23: la regla, no el caso).
/// Con un permiso opcional, olvidarlo en la ruta número catorce no produce nada
/// —ni error de compilación, ni prueba roja, ni aviso—; con este parámetro, una
/// ruta nueva NO COMPILA hasta que alguien decida qué permiso pide. Y las once
/// vistas de lectura declaran `Consultar` aunque hoy lo tengan los cuatro roles:
/// que hoy coincida es casualidad, y el día que nazca un rol sin `Consultar` se
/// abrirían solas.
///
/// Distingue además dos cosas que no son la misma: **sin sesión** se redirige a
/// la entrada; **con sesión y sin permiso** se responde 403. Mandar a la pantalla
/// de acceso a quien ya entró le hace creer que se le cayó la sesión, y se pone a
/// teclear su clave otra vez.
fn exigir(
    st: &Estado,
    h: &HeaderMap,
    p: usuario::Permiso,
) -> Result<crate::sesion::Sesion, Box<Response>> {
    let Some(s) = sesion_de(st, h) else {
        return Err(Box::new(respuesta_a_entrar()));
    };
    // `puede` devuelve `Option<bool>`: `None` es rol desconocido y NIEGA. Es la
    // única lectura segura de «no sé» en un control de acceso.
    if usuario::puede(&s.rol, p) != Some(true) {
        return Err(Box::new(prohibido()));
    }
    Ok(s)
}

// ── Artículos ─────────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct Busqueda {
    #[serde(default)]
    q: String,
    #[serde(default)]
    inactivos: bool,
    /// Artículo cuyo kardex se está mirando. Va en la URL para que la vista se
    /// pueda enlazar y volver a ella, igual que el rango de los informes.
    #[serde(default)]
    kardex: Option<i64>,
}

pub async fn articulos_pagina(
    State(st): State<Estado>,
    h: HeaderMap,
    Query(q): Query<Busqueda>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return *r;
    }
    let empresa = nombre_empresa(&st).await;
    let lista = datos_o_aviso!(&empresa, "los artículos", articulo::listar(&st.pool, &q.q, q.inactivos).await);

    envoltura("Artículos", &empresa, Some("/articulos"), html! {
        h2 { "Artículos" }
        // La búsqueda va por htmx contra el MISMO manejador, que devuelve solo
        // la tabla cuando se le pide un fragmento. Sin JavaScript propio: es
        // exactamente lo que htmx cubre.
        form.buscador hx-get="/articulos/tabla" hx-target="#tabla" hx-trigger="input changed delay:250ms, submit" {
            input type="search" name="q" value=(q.q) placeholder="Código, código de barras o descripción"
                  autocomplete="off";
            label.casilla {
                input type="checkbox" name="inactivos" value="true" checked[q.inactivos];
                " Incluir inactivos"
            }
        }
        div #tabla { (tabla_de_articulos(&lista)) }

        // El KARDEX del artículo. Es lo que hace auditable la existencia: la
        // regla 1 del repositorio dice que la existencia se DERIVA del kardex, y
        // sin poder verlo el usuario tiene que creerse el número —que es
        // exactamente lo que hacía el sistema viejo, con el kardex parado desde
        // 2004 y una existencia de ficción en pantalla—.
        h3 { "Kardex de un artículo" }
        form.buscador method="get" action="/articulos" {
            input type="hidden" name="q" value=(q.q);
            label { "Artículo"
                select name="kardex" {
                    option value="" { "— elija —" }
                    @for a in &lista {
                        option value=(a.id) selected[Some(a.id) == q.kardex] {
                            (a.codigo) " · " (a.descripcion)
                        }
                    }
                } }
            button.secundario type="submit" { "Ver kardex" }
        }
        @if let Some(id) = q.kardex {
            @match inventario_datos::kardex::historia(&st.pool, id, 200).await {
                Err(e) => p.aviso { "No se pudo leer el kardex: " (mostrable(&e)) }
                Ok(h) if h.is_empty() => p.vacio { "Ese artículo no tiene movimientos." }
                Ok(h) => {
                    div.tabla-ancha {
                        table.datos {
                            thead { tr {
                                th { "Fecha" } th { "Concepto" } th { "Documento" }
                                th.num { "Cantidad" } th.num { "Valor" }
                                th.num { "Existencia" } th.num { "Valor acumulado" }
                            } }
                            tbody {
                                @for f in &h {
                                    tr {
                                        td { (f.fecha) }
                                        td { (f.concepto) " (" (f.signo) ")" }
                                        td { (f.documento) }
                                        td.num { (unidades(f.cantidad)) }
                                        td.num { (pesos(f.valor)) }
                                        td.num { (unidades(f.existencia_despues)) }
                                        td.num { (pesos(f.valor_despues)) }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // El alta. `docs/OPERACION.md` la pone en el paso 6 del alistamiento
        // —«Artículos y terceros, o migrando»— y hasta hoy no existía en el
        // panel: una instalación sin corpus que migrar no podía empezar.
        h3 { "Nuevo artículo" }
        form.ficha method="post" action="/articulos" {
            div.rejilla {
                label { "Código" input name="codigo" required maxlength="20"; }
                label { "Descripción" input name="descripcion" required; }
                label { "Presentación" input name="presentacion" placeholder="Botellón 20 L"; }
                label { "Código de barras"
                    input name="codigo_barras" autocomplete="off"
                          placeholder="se valida el dígito GS1"; }
                label { "IVA (%)" input type="number" name="iva_porcentaje" min="0" max="100" value="19"; }
                label { "INC (%)" input type="number" name="inc_porcentaje" min="0" max="100" value="0"; }
                label { "Precio de venta (pesos)"
                    input type="number" name="precio_venta" min="0" step="1" value="0"; }
                label { "Punto mínimo" input type="number" name="punto_minimo" min="0" value="0"; }
                label { "Punto máximo" input type="number" name="punto_maximo" min="0" value="0"; }
                // La unidad de medida la EXIGE la factura electrónica por ítem
                // (Res. 000165/2023). No se rellena por omisión: el artículo
                // nace diciendo qué le falta.
                label { "Unidad de medida" input name="unidad_medida" placeholder="94, UN, KGM…"; }
                // Los dos datos con los que se mide el deterioro. Sin ellos el
                // corte de VNR informa el artículo como NO MEDIDO, que es la
                // verdad, en vez de suponer.
                label { "Precio de venta estimado (pesos)"
                    input type="number" name="precio_venta_estimado" min="0" step="1"
                          placeholder="para medir el deterioro"; }
                label { "Costos para vender (pesos)"
                    input type="number" name="costos_para_vender" min="0" step="1"
                          placeholder="para medir el deterioro"; }
            }
            p.nota {
                "El código de barras se valida contra su dígito de control GS1: si no lo pasa "
                "NO es un GTIN y se declara como código interno (999), que es la verdad. "
                "Sin unidad de medida ni estándar, el artículo no puede ir en una factura "
                "electrónica, y la tabla de arriba lo dice."
            }
            button.primario type="submit" { "Crear artículo" }
        }
    })
    .into_response()
}

/// Crea un artículo desde el panel.
pub async fn articulos_crear(State(st): State<Estado>, h: HeaderMap, cuerpo: String) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return *r;
    }
    let cs = campos(&cuerpo);
    // Un número ausente o ilegible NO es cero: en un precio, un cero por
    // omisión es un artículo regalado. Se rechaza y se dice cuál.
    let num = |c: &str| entero(uno(&cs, c));
    // `Ok(None)` = venía en blanco, que es legítimo; `Err(())` = venía algo y
    // NO era un número, que no lo es. Los dos casos son distintos y por eso no
    // se colapsan en `Option` (directiva 20): un precio estimado ilegible que
    // se tragara como «no hay» dejaría el artículo sin medir para el deterioro
    // y nadie se enteraría.
    let opcional_num = |c: &str| -> Result<Option<i64>, ()> {
        let v = uno(&cs, c).trim();
        if v.is_empty() { Ok(None) } else { entero(v).map(Some).ok_or(()) }
    };
    let texto_opcional = |c: &str| {
        let v = uno(&cs, c).trim().to_string();
        (!v.is_empty()).then_some(v)
    };

    let (Some(iva), Some(inc), Some(precio), Some(minimo), Some(maximo)) = (
        num("iva_porcentaje"), num("inc_porcentaje"), num("precio_venta"),
        num("punto_minimo"), num("punto_maximo"),
    ) else {
        return responder_articulos(&st, Some("El IVA, el INC, el precio y los puntos mínimo y máximo tienen que ser números.")).await;
    };
    let (estimado, costos) = match (opcional_num("precio_venta_estimado"), opcional_num("costos_para_vender")) {
        (Ok(a), Ok(b)) => (a, b),
        _ => return responder_articulos(&st, Some("El precio estimado y los costos para vender tienen que ser números.")).await,
    };

    // **LA COLUMNA ES CENTAVOS** (decisión de Juan, 2026-08-12). Lo declara la
    // migración —«el DINERO es INTEGER en CENTAVOS»—, lo escribe el migrador de
    // VFP y lo siembra la demo; el panel era el único que metía PESOS crudos, así
    // que un artículo creado aquí a 12.000 quedaba valiendo $120. Ninguna prueba
    // lo veía: las que hay comparan un número consigo mismo y pasan con las dos
    // convenciones.
    let (Some(precio), Some(estimado_c), Some(costos_c)) = (
        inventario_dominio::dinero::Centavos::intentar_de_pesos(precio),
        estimado.map_or(Some(None), |v| inventario_dominio::dinero::Centavos::intentar_de_pesos(v).map(Some)),
        costos.map_or(Some(None), |v| inventario_dominio::dinero::Centavos::intentar_de_pesos(v).map(Some)),
    ) else {
        return responder_articulos(&st, Some("Alguno de los importes es absurdo.")).await;
    };

    let n = articulo::Nuevo {
        codigo: uno(&cs, "codigo").to_string(),
        codigo_barras: texto_opcional("codigo_barras"),
        descripcion: uno(&cs, "descripcion").to_string(),
        presentacion: uno(&cs, "presentacion").to_string(),
        categoria_id: None,
        iva_porcentaje: iva,
        precio_venta: precio.0,
        punto_minimo: minimo,
        punto_maximo: maximo,
        inc_porcentaje: inc,
        unidad_medida: texto_opcional("unidad_medida"),
        precio_venta_estimado: estimado_c.map(|c| c.0),
        costos_para_vender: costos_c.map(|c| c.0),
    };
    match articulo::crear(&st.pool, &n).await {
        Ok(_) => responder_articulos(&st, Some("Artículo creado.")).await,
        Err(e) => responder_articulos(&st, Some(&format!("No se creó: {}", mostrable(&e)))).await,
    }
}

/// Redibuja Artículos con un dicho. Se recarga la lista, no se parchea: el
/// artículo nuevo tiene que aparecer en ella o no se creó.
async fn responder_articulos(st: &Estado, dicho: Option<&str>) -> Response {
    let empresa = nombre_empresa(st).await;
    let lista = datos_o_aviso!(&empresa, "los artículos", articulo::listar(&st.pool, "", false).await);
    envoltura("Artículos", &empresa, Some("/articulos"), html! {
        h2 { "Artículos" }
        @if let Some(m) = dicho { div.aviso-resultado role="status" { (m) } }
        div #tabla { (tabla_de_articulos(&lista)) }
        p.nota { a href="/articulos" { "Volver a Artículos" } }
    })
    .into_response()
}

/// El fragmento que htmx intercambia. Es el mismo listado, sin envoltura.
pub async fn articulos_tabla(
    State(st): State<Estado>,
    h: HeaderMap,
    Query(q): Query<Busqueda>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return *r;
    }
    let empresa = nombre_empresa(&st).await;
    let lista = datos_o_aviso!(&empresa, "los artículos", articulo::listar(&st.pool, &q.q, q.inactivos).await);
    tabla_de_articulos(&lista).into_response()
}

fn tabla_de_articulos(lista: &[articulo::Articulo]) -> Markup {
    html! {
        @if lista.is_empty() {
            p.vacio { "No hay artículos que encajen con la búsqueda." }
        } @else {
            table.datos {
                thead {
                    tr {
                        th { "Código" } th { "Descripción" } th { "Presentación" }
                        th.num { "Existencia" } th.num { "Precio" } th { "Estado" }
                    }
                }
                tbody {
                    @for a in lista {
                        tr.inactivo[!a.activo] {
                            td { code { (a.codigo) } }
                            td { (a.descripcion) }
                            td { (a.presentacion) }
                            // La existencia bajo el punto mínimo se marca: es el
                            // dato por el que se entra a esta pantalla.
                            td.num.alerta[a.existencia <= a.punto_minimo] {
                                (a.existencia)
                            }
                            // `/ 100` porque la columna es CENTAVOS, igual que
                            // `factura.total` unas líneas más abajo. Sin dividir,
                            // la tabla enseñaba el precio 100 veces alto.
                            td.num { (pesos(a.precio_venta / 100)) }
                            td { @if a.activo { "Activo" } @else { "Inactivo" } }
                        }
                    }
                }
            }
        }
    }
}

/// Los importes se guardan en pesos enteros —nunca en coma flotante—, así que
/// aquí solo se agrupan los miles. El redondeo es decisión del dominio, no de
/// la pantalla (decisión 0002).
fn pesos(v: i64) -> String {
    let negativo = v < 0;
    let s = v.abs().to_string();
    let mut salida = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            salida.push('.');
        }
        salida.push(c);
    }
    if negativo { format!("-{salida}") } else { salida }
}

// ── Raíz y ruta desconocida ───────────────────────────────────────────────────

/// La raíz manda a la primera sección si hay sesión, y a la entrada si no.
pub async fn raiz(State(st): State<Estado>, h: HeaderMap) -> Response {
    let destino = if sesion_de(&st, &h).is_some() { "/articulos" } else { "/entrar" };
    let mut cab = HeaderMap::new();
    cab.insert(header::LOCATION, destino.parse().expect("ruta válida"));
    (StatusCode::SEE_OTHER, cab).into_response()
}

/// Una ruta desconocida vuelve al inicio en vez de dejar la pantalla en blanco
/// — **salvo bajo `/api/`**, donde se responde 404. Un error de API disfrazado
/// de página es como un fallo acaba apareciendo en pantalla como «no pasa nada».
pub async fn no_encontrado(uri: axum::http::Uri) -> Response {
    if uri.path().starts_with("/api/") {
        return (StatusCode::NOT_FOUND, "no existe").into_response();
    }
    let mut cab = HeaderMap::new();
    cab.insert(header::LOCATION, "/".parse().expect("ruta válida"));
    (StatusCode::SEE_OTHER, cab).into_response()
}



// ── Alistamiento ──────────────────────────────────────────────────────────────

/// La pantalla que se abre el primer día. No dice «bienvenido»: dice qué falta
/// para poder trabajar, ordenado por lo que bloquea, y con qué hacer en cada
/// punto. Un asistente que solo enumera pasos obliga a recorrerlos todos aunque
/// ya estén hechos; éste mira el estado real y solo muestra lo que falta.
pub async fn alistamiento_pagina(State(st): State<Estado>, h: HeaderMap) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return *r;
    }
    let empresa = nombre_empresa(&st).await;
    let d = match inventario_datos::alistamiento::diagnosticar(&st.pool, &crate::rutas::hoy()).await {
        Ok(d) => d,
        Err(e) => return aviso(&empresa, "Alistamiento", &mostrable(&e)),
    };

    envoltura("Alistamiento", &empresa, Some("/alistamiento"), html! {
        h2 { "Alistamiento" }
        div.semaforos {
            (semaforo(d.puede_operar, "Se puede operar", "NO se puede operar",
                      "Facturar, mover inventario y recaudar."))
            (semaforo(d.puede_facturar_electronicamente,
                      "Factura electrónica lista", "Factura electrónica NO lista",
                      "CUFE, CUDE y los datos que exige la DIAN."))
        }

        @if d.puntos.is_empty() {
            p.todo-bien { "No queda nada pendiente. El sistema está listo para entregarse." }
        }
        @for p in &d.puntos {
            div class={ "punto " (clase_gravedad(p.gravedad)) } {
                // La etiqueta dice QUÉ BLOQUEA, no un nivel abstracto: «grave /
                // medio / leve» obliga a traducir mentalmente, «impide operar» no.
                div.etiqueta { (etiqueta_gravedad(p.gravedad)) }
                div.cuerpo {
                    b {
                        (p.que)
                        @if let Some(n) = p.cuantos { span.cuantos { " (" (n) ")" } }
                    }
                    p { (p.como) }
                }
            }
        }

        h3 { "Lo que ya hay" }
        div.resumen {
            (dato(d.inventario.articulos, "artículos"))
            (dato(d.inventario.terceros, "terceros"))
            (dato(d.inventario.usuarios_activos, "usuarios"))
            (dato(d.inventario.conceptos, "conceptos"))
            (dato(d.inventario.resoluciones_utilizables, "resoluciones usables"))
            (dato(d.inventario.movimientos, "movimientos"))
            (dato(d.inventario.facturas, "facturas"))
        }
    })
    .into_response()
}

fn semaforo(bien: bool, si: &str, no: &str, detalle: &str) -> Markup {
    html! {
        div class={ "tarjeta " (if bien { "bien" } else { "mal" }) } {
            // El símbolo va ACOMPAÑADO del texto, nunca solo: un icono verde y
            // uno rojo se ven igual para quien no distingue los dos colores.
            span.simbolo { (if bien { "✓" } else { "✕" }) }
            div { b { (if bien { si } else { no }) } p { (detalle) } }
        }
    }
}

fn clase_gravedad(g: inventario_datos::alistamiento::Gravedad) -> &'static str {
    use inventario_datos::alistamiento::Gravedad::*;
    match g {
        Bloqueante => "bloqueante",
        BloqueaFacturaElectronica => "fe",
        Advertencia => "aviso",
    }
}

fn etiqueta_gravedad(g: inventario_datos::alistamiento::Gravedad) -> &'static str {
    use inventario_datos::alistamiento::Gravedad::*;
    match g {
        Bloqueante => "Impide operar",
        BloqueaFacturaElectronica => "Impide facturar-e",
        Advertencia => "Conviene revisar",
    }
}

fn dato(n: i64, que: &str) -> Markup {
    html! { div { span { (n) } " " (que) } }
}

/// Traduce un error de la capa de datos a lo que se le puede ENSEÑAR a alguien.
///
/// `Error::Sql` NO sale a la pantalla: llevaría dentro nombres de tabla, de
/// columna y de restricción. `rutas::fallo` ya lo hacía para `/api` —con su
/// prueba— y este panel lo estaba deshaciendo en seis sitios, que es una
/// regresión de una salvaguarda ya decidida. El resto de errores SÍ salen
/// enteros: son reglas de negocio, y decirle al cajero «existencia insuficiente»
/// es justo lo que le deja arreglarlo.
fn mostrable(e: &inventario_datos::Error) -> String {
    match e {
        inventario_datos::Error::Sql(_) => {
            // Al registro sí va entero, que es donde hace falta para depurar.
            eprintln!("[error] sql en el panel: {e}");
            "error interno de la base de datos".to_string()
        }
        otro => otro.to_string(),
    }
}

/// Una pantalla que no pudo cargar sus datos lo DICE, con la envoltura puesta
/// para que se pueda navegar a otra parte. Devolver una página en blanco o un
/// 500 desnudo deja a quien está delante sin saber si el fallo es suyo.
fn aviso(empresa: &str, titulo: &str, mensaje: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        envoltura(titulo, empresa, Some(""), html! {
            h2 { (titulo) }
            div.error role="alert" { (mensaje) }
        }),
    )
        .into_response()
}

// ── Cartera ───────────────────────────────────────────────────────────────────

pub async fn cartera_pagina(State(st): State<Estado>, h: HeaderMap) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return *r;
    }
    let empresa = nombre_empresa(&st).await;
    let filas = match inventario_datos::factura::cartera(&st.pool, &crate::rutas::hoy()).await {
        Ok(v) => v,
        Err(e) => return aviso(&empresa, "Cartera", &mostrable(&e)),
    };
    let total: i64 = filas.iter().map(|f| f.saldo).sum();
    let vencido: i64 = filas.iter().map(|f| f.vencido).sum();

    envoltura("Cartera", &empresa, Some("/cartera"), html! {
        h2 { "Cartera" }
        div.resumen {
            (dato_pesos(total, "por cobrar"))
            (dato_pesos(vencido, "vencido"))
            (dato(filas.len() as i64, "clientes con saldo"))
        }
        @if filas.is_empty() {
            p.vacio { "No hay cartera pendiente." }
        } @else {
            div.tabla-ancha {
                table.datos {
                    thead { tr {
                        th { "Cliente" } th.num { "Facturas" }
                        th.num { "Saldo" } th.num { "Vencido" }
                    } }
                    tbody {
                        @for f in &filas {
                            tr {
                                td { (f.nombre) }
                                td.num { (f.facturas) }
                                td.num { (pesos(f.saldo)) }
                                // Lo vencido se marca solo cuando lo hay: pintar
                                // de rojo un cero enseña a no mirar el color.
                                td.num.alerta[f.vencido > 0] {
                                    (pesos(f.vencido))
                                }
                            }
                        }
                    }
                }
            }
        }
    })
    .into_response()
}

fn dato_pesos(v: i64, que: &str) -> Markup {
    html! { div { span { (pesos(v)) } " " (que) } }
}

/// Lo mismo, para magnitudes que van en unidades y no en pesos. El tope del
/// art. 64 se mide en UNIDADES —«las unidades del inventario final… de la suma
/// del inventario inicial más las compras»—, no en dinero, y enseñarlo con
/// formato de pesos invitaría a leerlo como un importe.
fn dato_unidades(v: i64, que: &str) -> Markup {
    html! { div { span { (unidades(v)) } " " (que) } }
}

// ── Terceros ──────────────────────────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct BusquedaTerceros {
    #[serde(default)]
    q: String,
    #[serde(default)]
    dicho: Option<String>,
}

pub async fn terceros_pagina(
    State(st): State<Estado>,
    h: HeaderMap,
    Query(q): Query<BusquedaTerceros>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return *r;
    }
    pantalla_terceros(&st, &q.q, q.dicho.as_deref()).await
}

async fn pantalla_terceros(st: &Estado, busqueda: &str, dicho: Option<&str>) -> Response {
    let empresa = nombre_empresa(st).await;
    let lista = datos_o_aviso!(&empresa, "los terceros", inventario_datos::tercero::listar(&st.pool, busqueda, None).await);

    envoltura("Terceros", &empresa, Some("/terceros"), html! {
        h2 { "Terceros" }
        @if let Some(m) = dicho { div.aviso-resultado role="status" { (m) } }

        form.buscador method="get" action="/terceros" {
            input type="search" name="q" value=(busqueda) placeholder="NIT o nombre" autocomplete="off";
            button.secundario type="submit" { "Buscar" }
        }

        @if lista.is_empty() {
            p.vacio { "No hay terceros que encajen con la búsqueda." }
        } @else {
            div.tabla-ancha {
                table.datos {
                    thead { tr {
                        th { "NIT" } th { "Nombre" } th { "Cliente" } th { "Proveedor" }
                        th { "Teléfono" } th.num { "Plazo (días)" } th { "Estado" }
                    } }
                    tbody {
                        @for t in &lista {
                            tr.inactivo[!t.activo] {
                                // El NIT se enseña CON su dígito de verificación:
                                // es lo que va en la factura, y uno escrito a
                                // mano que no corresponda lo rechaza la DIAN.
                                td { code { (t.nit_completo) } }
                                td { (t.nombre) }
                                td { @if t.es_cliente { "Sí" } @else { "—" } }
                                td { @if t.es_proveedor { "Sí" } @else { "—" } }
                                td { (t.telefono) }
                                td.num { (t.plazo_dias) }
                                td { @if t.activo { "Activo" } @else { "Inactivo" } }
                            }
                        }
                    }
                }
            }
        }

        h3 { "Nuevo tercero" }
        form.ficha method="post" action="/terceros" {
            div.rejilla {
                // El DV no se teclea, igual que en la empresa: lo calcula el
                // dominio a partir del NIT.
                label { "NIT o cédula" input name="nit" required placeholder="sin dígito de verificación"; }
                label { "Nombre" input name="nombre" required; }
                label { "Dirección" input name="direccion"; }
                label { "Teléfono" input name="telefono"; }
                label { "Correo" input type="email" name="correo"; }
                label { "Plazo de crédito (días)"
                    input type="number" name="plazo_dias" min="0" value="0"; }
                label.casilla { input type="checkbox" name="es_cliente" value="true" checked; " Cliente" }
                label.casilla { input type="checkbox" name="es_proveedor" value="true"; " Proveedor" }
            }
            p.nota {
                "Un tercero es cliente, proveedor o las dos cosas: si no se marca ninguna, "
                "no se crea. El dígito de verificación lo calcula el sistema."
            }
            button.primario type="submit" { "Crear tercero" }
        }
    })
    .into_response()
}

pub async fn terceros_crear(State(st): State<Estado>, h: HeaderMap, cuerpo: String) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return *r;
    }
    let cs = campos(&cuerpo);
    // Una casilla sin marcar no llega en el cuerpo: su ausencia SÍ significa
    // «false», y eso lo define el navegador, así que aquí es una lectura
    // correcta y no un relleno por omisión.
    let n = inventario_datos::tercero::Nuevo {
        nit: uno(&cs, "nit").to_string(),
        nombre: uno(&cs, "nombre").to_string(),
        es_cliente: !uno(&cs, "es_cliente").is_empty(),
        es_proveedor: !uno(&cs, "es_proveedor").is_empty(),
        direccion: uno(&cs, "direccion").to_string(),
        telefono: uno(&cs, "telefono").to_string(),
        telefono2: String::new(),
        correo: uno(&cs, "correo").to_string(),
        ciudad_id: None,
        zona_id: None,
        barrio_id: None,
        vendedor_id: None,
        convenio_id: None,
        // El plazo ilegible NO es cero: cero es «de contado», que es una
        // condición comercial distinta de «no se entendió lo que escribieron».
        plazo_dias: match entero(uno(&cs, "plazo_dias")) {
            Some(p) => p,
            None if uno(&cs, "plazo_dias").trim().is_empty() => 0,
            None => {
                return pantalla_terceros(&st, "", Some("El plazo tiene que ser un número de días.")).await;
            }
        },
    };
    match inventario_datos::tercero::crear(&st.pool, &n).await {
        Ok(_) => pantalla_terceros(&st, "", Some("Tercero creado.")).await,
        Err(e) => pantalla_terceros(&st, "", Some(&format!("No se creó: {}", mostrable(&e)))).await,
    }
}

// ── Informes ──────────────────────────────────────────────────────────────────

/// Rango de fechas de los informes, con un defecto que NO inventa datos: el
/// mes corriente. Es un encuadre de consulta, no un dato del negocio — si
/// alguien pide otro, lo pide (directiva 20 no obliga a fallar ante la ausencia
/// de un filtro, obliga a no rellenar un HECHO).
#[derive(Deserialize, Default)]
pub struct RangoInformes {
    #[serde(default)]
    desde: Option<String>,
    #[serde(default)]
    hasta: Option<String>,
    /// Fecha del corte de deterioro. Va aparte del rango: la NIC 2 mide el VNR
    /// a una FECHA, no a lo largo de un periodo (directiva 32).
    #[serde(default)]
    corte: Option<String>,
    #[serde(default)]
    dias_reposicion: Option<i64>,
    #[serde(default)]
    nivel_servicio: Option<i64>,
    /// Lo que el último POST tenga que decir, para no perderlo al redirigir.
    #[serde(default)]
    dicho: Option<String>,
}

fn primero_del_mes(hoy: &str) -> String {
    // `hoy` sale de `crate::rutas::hoy()`, que siempre da AAAA-MM-DD. Si aun
    // así no se pudiera leer, se dice en vez de fabricar una fecha.
    use chrono::Datelike;
    match chrono::NaiveDate::parse_from_str(hoy, "%Y-%m-%d") {
        Ok(d) => d.with_day(1).unwrap_or(d).to_string(),
        Err(_) => hoy.to_string(),
    }
}

/// Un porcentaje guardado en centésimas de punto: `250` → `2,50 %`.
fn porcentaje(centesimas: i64) -> String {
    let negativo = centesimas < 0;
    let a = centesimas.abs();
    format!("{}{},{:02} %", if negativo { "-" } else { "" }, a / 100, a % 100)
}

/// Unidades guardadas en milésimas: `1250` → `1,250`.
fn unidades(milesimas: i64) -> String {
    if milesimas % 1000 == 0 {
        return pesos(milesimas / 1000);
    }
    let negativo = milesimas < 0;
    let a = milesimas.abs();
    format!("{}{},{:03}", if negativo { "-" } else { "" }, pesos(a / 1000), a % 1000)
}

pub async fn informes_pagina(
    State(st): State<Estado>,
    h: HeaderMap,
    Query(q): Query<RangoInformes>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return *r;
    }
    pantalla_informes(&st, &q).await
}

async fn pantalla_informes(st: &Estado, q: &RangoInformes) -> Response {
    let empresa = nombre_empresa(st).await;
    let hoy = crate::rutas::hoy();
    let desde = q.desde.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| primero_del_mes(&hoy));
    let hasta = q.hasta.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| hoy.clone());
    let corte = q.corte.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| hoy.clone());
    let dias_reposicion = q.dias_reposicion.filter(|d| *d > 0).unwrap_or(7);
    let nivel_servicio = q.nivel_servicio.unwrap_or(95);

    let escasos = datos_o_aviso!(&empresa, "los artículos bajo mínimo", inventario_datos::kardex::bajo_minimo(&st.pool).await);

    // **Un informe que no se pudo calcular lo DICE.** No se dibuja una tabla
    // vacía, que es indistinguible de «no hubo movimientos» (directiva 30): el
    // error del dominio —una fecha ilegible, un nivel de servicio que no está
    // tabulado— se enseña donde iba el informe.
    let art64 = inventario_datos::tributario::art64(&st.pool, &desde, &hasta).await;
    let retiros = inventario_datos::tributario::retiros_gravados(&st.pool, &desde, &hasta).await;
    let rotacion = inventario_datos::gestion::rotacion(&st.pool, &desde, &hasta).await;
    let abc = inventario_datos::gestion::abc(&st.pool, &desde, &hasta).await;
    let reorden = inventario_datos::gestion::punto_reorden(
        &st.pool, &desde, &hasta, dias_reposicion, nivel_servicio,
    )
    .await;
    let deterioro = inventario_datos::deterioro::leer_corte(&st.pool, &corte).await;
    let pendientes = inventario_datos::tributario::bajas_sin_acta(&st.pool, &desde, &hasta).await;

    envoltura("Informes", &empresa, Some("/informes"), html! {
        h2 { "Informes" }
        @if let Some(m) = q.dicho.as_deref().filter(|m| !m.is_empty()) {
            div.aviso-resultado role="status" { (m) }
        }

        // El rango va en la URL (GET), no en la sesión: así un informe se puede
        // guardar como enlace y se puede volver a él, que es lo que hace quien
        // arma una declaración.
        form.ficha method="get" action="/informes" {
            div.rejilla {
                label { "Desde" input type="date" name="desde" value=(desde); }
                label { "Hasta" input type="date" name="hasta" value=(hasta); }
                label { "Corte de deterioro" input type="date" name="corte" value=(corte); }
                label { "Días de reposición"
                    input type="number" name="dias_reposicion" min="1" value=(dias_reposicion); }
                label { "Nivel de servicio (%)"
                    select name="nivel_servicio" {
                        @for n in [50i64, 80, 85, 90, 95, 98, 99] {
                            option value=(n) selected[n == nivel_servicio] { (n) " %" }
                        }
                    } }
            }
            button.primario type="submit" { "Ver el periodo" }
        }

        // ── ET art. 64: el tope del 3 % ──────────────────────────────────────
        h3 { "Disminución del inventario — tope del 3 % (ET art. 64)" }
        @match &art64 {
            Err(e) => p.aviso { "No se pudo calcular: " (mostrable(e)) }
            Ok(i) => {
                @if let Some(a) = &i.advertencia { p.aviso { b { (a) } } }
                div.semaforos {
                    (dato_unidades(i.base, "base (inicial + compras)"))
                    (dato_unidades(i.tope, "tope del 3 %"))
                    (dato_unidades(i.consumido, "consumido"))
                    (dato_unidades(i.disponible, "disponible"))
                }
                p { "Consumido: " b { (porcentaje(i.porcentaje_consumido)) } " del tope."
                    @if i.se_paso {
                        " " b.alerta { "SE PASÓ en " (unidades(i.exceso)) " unidades." }
                        " Por encima del 3 % hay que demostrar fuerza mayor o caso fortuito."
                    } }
                @if i.detalle.is_empty() {
                    p.vacio { "Ningún faltante consumió el tope en el periodo." }
                } @else {
                    div.tabla-ancha {
                        table.datos {
                            thead { tr {
                                th { "Fecha" } th { "Artículo" } th { "Concepto" }
                                th.num { "Cantidad" } th.num { "Costo" }
                            } }
                            tbody {
                                @for f in &i.detalle {
                                    tr {
                                        td { (f.fecha) }
                                        td { (f.articulo) }
                                        td { (f.concepto) }
                                        td.num { (unidades(f.cantidad)) }
                                        td.num { (pesos(f.costo)) }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── El acta que sustenta la baja ────────────────────────────────────
        //
        // Va JUNTO al informe del art. 64 a propósito: es ahí donde uno
        // descubre que hace falta un acta, y una pantalla aparte obliga a
        // acordarse. La norma enumera los campos —cantidad, descripción, costo
        // fiscal unitario, justificación y firmas— y sin ellos la deducción no
        // se sustenta.
        h4 { "Acta de baja por obsolescencia (ET art. 64)" }
        @match &pendientes {
            Err(e) => p.aviso { "No se pudieron leer las bajas: " (mostrable(e)) }
            Ok(ps) if ps.is_empty() => {
                p.vacio {
                    "No hay bajas por obsolescencia sin acta en el periodo. "
                    "El acta se levanta sobre la salida que YA ocurrió: primero se registra "
                    "el movimiento con un concepto de efecto «baja con acta», y aquí aparece."
                }
            }
            Ok(ps) => {
                // El acta se arma sobre las salidas que ya ocurrieron, y por
                // eso se ELIGEN de una lista en vez de teclearse: `crear_acta`
                // exige el movimiento de kardex que sacó la mercancía, y la
                // descripción y el costo fiscal unitario los deriva de él. Un
                // acta que dice un costo y un kardex que dice otro no sustenta
                // nada, así que aquí no hay dónde teclear un costo.
                form.ficha method="post" action="/informes/acta" {
                    input type="hidden" name="desde" value=(desde);
                    input type="hidden" name="hasta" value=(hasta);
                    div.tabla-ancha {
                        table.datos {
                            thead { tr {
                                th { "Incluir" } th { "Fecha" } th { "Artículo" }
                                th.num { "Pendiente" } th.num { "Costo fiscal unitario" }
                                th { "Justificación" }
                            } }
                            tbody {
                                @for b in ps {
                                    tr {
                                        td {
                                            input type="checkbox" name="movimiento"
                                                  value=(b.movimiento_id);
                                        }
                                        td { (b.fecha) }
                                        td { (b.articulo) }
                                        td.num {
                                            (unidades(b.pendiente))
                                            @if b.con_acta > 0 {
                                                " (de " (unidades(b.cantidad)) ")"
                                            }
                                        }
                                        // `/100`: `costo_fiscal_unitario` es `Centavos`. Al arreglar el
                            // acuse dejé ESTE campo crudo, y con eso el formulario
                            // contradecía a su propio acuse por 100× — «10 unidades ×
                            // 300.000» dando «costo total 30.000». Antes los dos
                            // mentían igual; el arreglo a medias fue peor que ninguno.
                            td.num { (pesos(b.costo_fiscal_unitario / 100)) }
                                        td { input name="justificacion" placeholder="por qué se dio de baja"; }
                                    }
                                }
                            }
                        }
                    }
                    div.rejilla {
                        label { "Fecha del acta" input type="date" name="fecha" value=(hasta) required; }
                        label { "Método"
                            select name="metodo" required {
                                option value="" { "— elija —" }
                                @for m in ["destruccion", "reciclaje", "chatarrizacion"] {
                                    option value=(m) { (m) }
                                }
                            } }
                        label { "Cubierto por seguro (pesos)"
                            input type="number" name="cubierto_por_seguro" min="0" step="1" value="0"; }
                        label { "Representante legal" input name="representante_legal" required; }
                        label { "Responsables (separados por «;»)"
                            input name="responsables" required; }
                    }
                    p.nota {
                        "La norma exige cantidad, descripción, costo fiscal unitario, justificación "
                        "y firmas, y que los bienes se destruyan, reciclen o chatarricen. "
                        "Solo es deducible lo NO cubierto por el seguro."
                    }
                    button.primario type="submit" { "Levantar el acta" }
                }
            }
        }

        // ── ET art. 421 lit. b): retiros que causan IVA ──────────────────────
        h3 { "Retiros gravados con IVA (ET art. 421 lit. b)" }
        @match &retiros {
            Err(e) => p.aviso { "No se pudo calcular: " (mostrable(e)) }
            Ok(i) => {
                @if i.filas.is_empty() {
                    p.vacio {
                        "Ningún retiro gravado en el periodo. "
                        "Si hubo obsequios o consumo interno, revise el efecto tributario de "
                        "sus conceptos en Configuración: un concepto neutro no liquida IVA."
                    }
                } @else {
                    div.semaforos {
                        (dato_pesos(i.base_total, "base (valor comercial)"))
                        (dato_pesos(i.iva_total, "IVA generado"))
                        (dato_pesos(i.costo_total, "costo de lo retirado"))
                    }
                    div.tabla-ancha {
                        table.datos {
                            thead { tr {
                                th { "Fecha" } th { "Artículo" } th { "Concepto" }
                                th.num { "Cantidad" } th.num { "Costo" }
                                th.num { "Valor comercial" } th.num { "IVA" }
                            } }
                            tbody {
                                @for f in &i.filas {
                                    tr {
                                        td { (f.fecha) }
                                        td { (f.articulo) }
                                        td { (f.concepto) }
                                        td.num { (unidades(f.cantidad)) }
                                        td.num { (pesos(f.costo)) }
                                        td.num { (pesos(f.valor_comercial)) }
                                        td.num { (pesos(f.iva)) }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── NIC 2.28-33: deterioro por VNR ──────────────────────────────────
        h3 { "Deterioro por valor neto realizable — corte al " (corte) " (NIC 2.28–33)" }
        // Correr el corte ESCRIBE, así que es un POST aparte y no se dispara
        // por mirar la pantalla. Reevaluar es lo que manda la NIC 2.33 y es
        // idempotente por fecha.
        form.ficha method="post" action="/informes/deterioro" {
            input type="hidden" name="fecha" value=(corte);
            p.nota {
                "Medir al menor entre costo y VNR. Correrlo de nuevo el mismo día "
                "reemplaza la medición de ese día; no la duplica."
            }
            button.primario type="submit" { "Correr el corte al " (corte) }
        }
        @match &deterioro {
            Err(e) => p.aviso { "No se pudo leer: " (mostrable(e)) }
            Ok(filas) if filas.is_empty() => {
                p.vacio { "No hay corte corrido a esta fecha." }
            }
            Ok(filas) => {
                @let rebajas: i64 = filas.iter().filter(|f| f.movimiento > 0).map(|f| f.movimiento).sum();
                @let reversiones: i64 = filas.iter().filter(|f| f.movimiento < 0).map(|f| -f.movimiento).sum();
                // La revelación de la NIC 2.36 pide rebajas y reversiones
                // SEPARADAS, no su neto: compensarlas esconde las dos.
                div.semaforos {
                    (dato_pesos(filas.iter().map(|f| f.costo).sum(), "costo"))
                    (dato_pesos(filas.iter().map(|f| f.valor_en_libros).sum(), "valor en libros"))
                    (dato_pesos(rebajas, "rebajas del periodo"))
                    (dato_pesos(reversiones, "reversiones del periodo"))
                }
                div.tabla-ancha {
                    table.datos {
                        thead { tr {
                            th { "Código" } th { "Descripción" } th.num { "Existencia" }
                            th.num { "Costo" } th.num { "VNR" } th.num { "En libros" }
                            th.num { "Deterioro acum." } th { "Medido" }
                        } }
                        tbody {
                            @for f in filas {
                                tr {
                                    td { code { (f.codigo) } }
                                    td { (f.descripcion) }
                                    td.num { (unidades(f.existencia)) }
                                    td.num { (pesos(f.costo)) }
                                    td.num { (pesos(f.vnr)) }
                                    td.num { (pesos(f.valor_en_libros)) }
                                    td.num.alerta[f.deterioro_acumulado > 0] { (pesos(f.deterioro_acumulado)) }
                                    // «No medido» NO es cero: es que falta el
                                    // precio de venta estimado del artículo.
                                    td { @if f.medido { "Sí" } @else { "NO — falta precio estimado" } }
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── Gestión ─────────────────────────────────────────────────────────
        h3 { "Bajo el punto mínimo" }
        @if escasos.is_empty() {
            p.vacio { "Ningún artículo está bajo su punto mínimo." }
        } @else {
            div.tabla-ancha {
                table.datos {
                    thead { tr {
                        th { "Código" } th { "Descripción" } th.num { "Existencia" }
                        th.num { "Mínimo" } th.num { "Máximo" } th.num { "Sugerido" }
                    } }
                    tbody {
                        @for e in &escasos {
                            tr {
                                td { code { (e.codigo) } }
                                td { (e.descripcion) }
                                td.num.alerta[true] { (e.existencia) }
                                td.num { (e.punto_minimo) }
                                td.num { (e.punto_maximo) }
                                td.num { (e.sugerido) }
                            }
                        }
                    }
                }
            }
        }

        h3 { "Punto de reorden — " (nivel_servicio) " % de servicio, " (dias_reposicion) " días de reposición" }
        @match &reorden {
            Err(e) => p.aviso { "No se pudo calcular: " (mostrable(e)) }
            Ok(filas) if filas.is_empty() => p.vacio { "Sin datos de demanda en el periodo." }
            Ok(filas) => {
                div.tabla-ancha {
                    table.datos {
                        thead { tr {
                            th { "Código" } th { "Descripción" } th.num { "Existencia" }
                            th.num { "Demanda/día" } th.num { "Desviación" }
                            th.num { "Stock seguridad" } th.num { "Punto de reorden" }
                            th.num { "Sugerido" } th.num { "Días de historia" } th { "¿Pedir?" }
                        } }
                        tbody {
                            @for f in filas {
                                tr {
                                    td { code { (f.codigo) } }
                                    td { (f.descripcion) }
                                    td.num { (unidades(f.existencia)) }
                                    td.num { (unidades(f.demanda_diaria)) }
                                    td.num { (unidades(f.desviacion)) }
                                    td.num { (unidades(f.stock_seguridad)) }
                                    td.num { (unidades(f.punto_reorden)) }
                                    td.num { (unidades(f.sugerido)) }
                                    // Los días de historia van en la tabla
                                    // porque un punto de reorden sobre tres
                                    // días no vale lo que uno sobre noventa.
                                    td.num { (f.dias_de_historia) }
                                    td.alerta[f.reponer] { @if f.reponer { "SÍ" } @else { "no" } }
                                }
                            }
                        }
                    }
                }
            }
        }

        h3 { "Rotación y días de inventario" }
        @match &rotacion {
            Err(e) => p.aviso { "No se pudo calcular: " (mostrable(e)) }
            Ok(filas) if filas.is_empty() => p.vacio { "Sin ventas en el periodo." }
            Ok(filas) => {
                div.tabla-ancha {
                    table.datos {
                        thead { tr {
                            th { "Código" } th { "Descripción" } th.num { "Costo de ventas" }
                            th.num { "Inventario promedio" } th.num { "Rotación" }
                            th.num { "Días de inventario" }
                        } }
                        tbody {
                            @for f in filas {
                                tr {
                                    td { code { (f.codigo) } }
                                    td { (f.descripcion) }
                                    td.num { (pesos(f.costo_de_ventas)) }
                                    td.num { (pesos(f.inventario_promedio)) }
                                    td.num { (porcentaje(f.rotacion_centesimas)) }
                                    // «No rotó» no es cero días: es que dividir
                                    // entre cero no da un dato que nadie pueda
                                    // usar. Se dice.
                                    td.num {
                                        @match f.dias_inventario {
                                            Some(d) => (d),
                                            None => "no rotó",
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        h3 { "Clasificación ABC por valor consumido" }
        @match &abc {
            Err(e) => p.aviso { "No se pudo calcular: " (mostrable(e)) }
            Ok(filas) if filas.is_empty() => p.vacio { "Sin consumo en el periodo." }
            Ok(filas) => {
                div.tabla-ancha {
                    table.datos {
                        thead { tr {
                            th { "Clase" } th { "Código" } th { "Descripción" }
                            th.num { "Valor consumido" } th.num { "Participación" }
                            th.num { "Acumulado" }
                        } }
                        tbody {
                            @for f in filas {
                                tr {
                                    td { b { (f.clase) } }
                                    td { code { (f.codigo) } }
                                    td { (f.descripcion) }
                                    td.num { (pesos(f.valor_consumido)) }
                                    td.num { (porcentaje(f.participacion)) }
                                    td.num { (porcentaje(f.acumulado)) }
                                }
                            }
                        }
                    }
                }
            }
        }
    })
    .into_response()
}

// ── Formularios con líneas: el parseo que htmx no trae ────────────────────────
//
// Facturar y recaudar tienen un número VARIABLE de renglones, y htmx los añade
// pidiendo un fragmento al servidor (`hx-swap="beforeend"`). Al enviar, el
// cuerpo trae claves repetidas —`articulo=1&articulo=2&…`—, y `axum::Form` no
// las junta en un `Vec`: `serde_urlencoded` se queda con la última y el resto
// desaparece EN SILENCIO, que es el peor resultado posible (directiva 20).
//
// Por eso el cuerpo se lee crudo y se parte aquí. Sin dependencia nueva y con
// la regla a la vista: cada clave devuelve TODOS sus valores, en orden.
fn campos(cuerpo: &str) -> Vec<(String, String)> {
    cuerpo
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|par| {
            // Un par SIN `=` es una clave con valor vacío, no basura: lo dice el
            // algoritmo de WHATWG para `application/x-www-form-urlencoded`.
            // Con `filter_map` + `?` desaparecía en silencio, así que una casilla
            // que el navegador mandara sin valor se leía como «no vino», que es
            // el relleno que la directiva 20 prohíbe.
            match par.split_once('=') {
                Some((k, v)) => (descodificar(k), descodificar(v)),
                None => (descodificar(par), String::new()),
            }
        })
        .collect()
}

/// `application/x-www-form-urlencoded`: `+` es espacio y `%XX` es un byte.
fn descodificar(s: &str) -> String {
    let bytes = s.replace('+', " ").into_bytes();
    let mut salida = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let alto = (bytes[i + 1] as char).to_digit(16);
            let bajo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(a), Some(b)) = (alto, bajo) {
                salida.push((a * 16 + b) as u8);
                i += 3;
                continue;
            }
        }
        salida.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&salida).into_owned()
}

/// El primer valor de una clave, o vacío. Para los campos que van una sola vez.
fn uno<'a>(cs: &'a [(String, String)], clave: &str) -> &'a str {
    cs.iter().find(|(k, _)| k == clave).map(|(_, v)| v.as_str()).unwrap_or("")
}

/// Todos los valores de una clave, en el orden en que llegaron. Es lo que
/// permite casar `articulo[3]` con `cantidad[3]` sin numerarlos en el HTML.
fn todos<'a>(cs: &'a [(String, String)], clave: &str) -> Vec<&'a str> {
    cs.iter().filter(|(k, _)| k == clave).map(|(_, v)| v.as_str()).collect()
}

/// Un entero de un campo de formulario. **Ausente o ilegible NO es cero**: es
/// `None`, y quien llama decide si eso es un error (directiva 20). Un cero por
/// omisión en una factura es una línea regalada.
///
/// **Rechaza, no limpia.** La primera versión filtraba a dígitos y signo antes
/// de parsear, y eso no es tolerancia: es FABRICAR un número donde había basura.
/// Medido: `"12abc34"` daba `Some(1234)` y `"1.234,50"` daba `Some(123450)` —un
/// importe multiplicado por cien—; un identificador con ruido resolvía a OTRO
/// identificador existente, o sea a otra resolución o a otro tercero. Ahora solo
/// pasa lo que es de verdad un entero, con su signo opcional y nada más.
fn entero(v: &str) -> Option<i64> {
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    let cuerpo = v.strip_prefix('-').unwrap_or(v);
    if cuerpo.is_empty() || !cuerpo.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // `parse` es quien decide el desbordamiento: `9223372036854775808` no cabe
    // en i64 y sale `None`, que es lo correcto.
    v.parse().ok()
}

// ── Movimientos de inventario ─────────────────────────────────────────────────

pub async fn movimientos_pagina(State(st): State<Estado>, h: HeaderMap) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return *r;
    }
    let empresa = nombre_empresa(&st).await;
    formulario_movimiento(&st, &empresa, None).await
}

async fn formulario_movimiento(st: &Estado, empresa: &str, dicho: Option<&str>) -> Response {
    let conceptos = datos_o_aviso!(empresa, "los conceptos", catalogo::conceptos(&st.pool).await);
    let articulos = datos_o_aviso!(empresa, "los artículos", articulo::listar(&st.pool, "", false).await);
    let terceros = datos_o_aviso!(
        empresa,
        "los terceros",
        inventario_datos::tercero::listar(&st.pool, "", None).await
    );

    envoltura("Movimiento de inventario", empresa, Some("/movimientos"), html! {
        h2 { "Movimiento de inventario" }
        @if let Some(m) = dicho { div.aviso-resultado role="status" { (m) } }
        form.ficha method="post" action="/movimientos" {
            div.rejilla {
                label { "Fecha"
                    input type="date" name="fecha" value=(crate::rutas::hoy()) required; }
                label { "Concepto"
                    select name="concepto_id" required {
                        option value="" { "— elija —" }
                        @for c in &conceptos {
                            @if c.activo {
                                // El signo va EN LA ETIQUETA: «salida» y «entrada»
                                // se distinguen aquí, no después de guardar.
                                option value=(c.id) {
                                    (c.codigo) " · " (c.nombre) " (" (c.signo) ")"
                                }
                            }
                        }
                    } }
                label { "Artículo"
                    select name="articulo_id" required {
                        option value="" { "— elija —" }
                        @for a in &articulos {
                            option value=(a.id) { (a.codigo) " · " (a.descripcion) }
                        }
                    } }
                label { "Cantidad (unidades)"
                    input type="number" name="cantidad" min="1" step="1" required; }
                label { "Costo unitario (pesos)"
                    input type="number" name="costo" min="0" step="1" value="0"; }
                label { "Documento"
                    input name="documento" maxlength="20"; }
                label { "Tercero (si el concepto lo exige)"
                    select name="tercero_id" {
                        option value="" { "— ninguno —" }
                        @for t in &terceros { option value=(t.id) { (t.nombre) } }
                    } }
                label { "Valor comercial unitario (solo retiro gravado)"
                    input type="number" name="valor_comercial" min="0" step="1"; }
            }
            button.primario type="submit" { "Registrar movimiento" }
        }
        p.nota {
            "El costo de salida lo fija el promedio: en una salida, «costo unitario» se ignora."
        }
    })
    .into_response()
}

pub async fn movimientos_enviar(State(st): State<Estado>, h: HeaderMap, cuerpo: String) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::MoverInventario) {
        return *r;
    }
    let s = sesion_de(&st, &h);
    let empresa = nombre_empresa(&st).await;
    let cs = campos(&cuerpo);

    // Los tres obligatorios se exigen; ausentes NO se rellenan con un defecto.
    let (Some(concepto_id), Some(articulo_id), Some(unidades)) = (
        entero(uno(&cs, "concepto_id")),
        entero(uno(&cs, "articulo_id")),
        entero(uno(&cs, "cantidad")),
    ) else {
        return formulario_movimiento(&st, &empresa, Some("Faltan concepto, artículo o cantidad.")).await;
    };

    // Lo que teclea quien opera pasa por la conversión que PUEDE fallar: en
    // el `×100` desborda; con `overflow-checks = true` en release eso es pánico
    // en los dos perfiles, o sea un 500 en la cara del cajero. Se rechaza antes.
    use inventario_dominio::dinero::{Cantidad, Centavos};
    let (Some(cantidad), Some(costo_unitario)) = (
        Cantidad::intentar_de_unidades(unidades),
        Centavos::intentar_de_pesos(entero(uno(&cs, "costo")).unwrap_or(0)),
    ) else {
        return formulario_movimiento(&st, &empresa, Some("La cantidad o el costo son absurdos.")).await;
    };
    let comercial = match entero(uno(&cs, "valor_comercial")) {
        Some(v) => match inventario_dominio::dinero::Centavos::intentar_de_pesos(v) {
            Some(c) => Some(c),
            None => {
                return formulario_movimiento(&st, &empresa, Some("El valor comercial es absurdo.")).await;
            }
        },
        None => None,
    };

    let p = inventario_datos::kardex::Peticion {
        fecha: uno(&cs, "fecha").to_string(),
        concepto_id,
        articulo_id,
        tercero_id: entero(uno(&cs, "tercero_id")),
        documento: uno(&cs, "documento").to_string(),
        cantidad,
        costo_unitario,
        factura_id: None,
        usuario_id: s.map(|s| s.usuario_id),
        valor_comercial_unitario: comercial,
    };

    match inventario_datos::kardex::registrar(&st.pool, &p).await {
        Ok(id) => formulario_movimiento(&st, &empresa, Some(&format!("Movimiento {id} registrado."))).await,
        // El motivo del rechazo VUELVE a la pantalla. Un «no se pudo» sin más
        // deja a quien lo intenta probando a ciegas contra su propio sistema.
        Err(e) => formulario_movimiento(&st, &empresa, Some(&format!("No se registró: {}", mostrable(&e)))).await,
    }
}

// ── Facturar ──────────────────────────────────────────────────────────────────

pub async fn facturar_pagina(State(st): State<Estado>, h: HeaderMap) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return *r;
    }
    let empresa = nombre_empresa(&st).await;
    formulario_factura(&st, &empresa, None).await
}

async fn formulario_factura(st: &Estado, empresa: &str, dicho: Option<&str>) -> Response {
    let resoluciones = datos_o_aviso!(
        empresa,
        "las resoluciones de facturación",
        inventario_datos::resolucion::listar(&st.pool, &crate::rutas::hoy()).await
    );
    let terceros = datos_o_aviso!(empresa, "los clientes", inventario_datos::tercero::listar(&st.pool, "", Some("cliente")).await);
    let articulos = datos_o_aviso!(
        empresa,
        "los artículos",
        articulo::listar(&st.pool, "", false).await
    );

    envoltura("Facturar", empresa, Some("/facturar"), html! {
        h2 { "Facturar" }
        @if let Some(m) = dicho { div.aviso-resultado role="status" { (m) } }

        // Una resolución con impedimento se muestra DESHABILITADA y con el
        // motivo, en vez de esconderse: si desaparece, quien factura no sabe si
        // no existe o si algo la inhabilitó (directiva 20).
        form.ficha method="post" action="/facturar" {
            div.rejilla {
                label { "Resolución"
                    select name="resolucion_id" required {
                        option value="" { "— elija —" }
                        @for r in &resoluciones {
                            @if let Some(motivo) = &r.impedimento {
                                option value=(r.id) disabled {
                                    (r.prefijo) " " (r.numero) " — no usable: " (motivo)
                                }
                            } @else {
                                option value=(r.id) {
                                    (r.prefijo) " " (r.numero) " (quedan " (r.quedan) ")"
                                }
                            }
                        }
                    } }
                label { "Cliente"
                    select name="tercero_id" required {
                        option value="" { "— elija —" }
                        @for t in &terceros { option value=(t.id) { (t.nit_completo) " · " (t.nombre) } }
                    } }
                label { "Fecha"
                    input type="date" name="fecha" value=(crate::rutas::hoy()) required; }
                label { "Forma de pago"
                    select name="forma_pago" required {
                        option value="contado" { "Contado" }
                        option value="credito" { "Crédito" }
                    } }
                label { "Medio de pago (obligatorio en contado)"
                    select name="medio_pago" {
                        option value="" { "— ninguno (crédito) —" }
                        @for (valor, etiqueta) in inventario_dominio::factura::MEDIOS_DE_PAGO {
                            option value=(valor) { (etiqueta) }
                        }
                    } }
                label { "Observación"
                    input name="observacion" maxlength="200"; }
            }

            h3 { "Líneas" }
            // Las líneas las añade htmx pidiendo un fragmento al servidor: el
            // HTML de una línea se teje en UN solo sitio, y así la de la fila 7
            // no puede divergir de la de la fila 1.
            table.datos #lineas {
                thead { tr {
                    th { "Artículo" } th.num { "Cantidad" } th.num { "Precio unitario" }
                    th.num { "Descuento" } th.num { "IVA" }
                } }
                tbody #cuerpo-lineas { (linea_de_factura(&articulos)) }
            }
            button.secundario type="button"
                   hx-get="/facturar/linea" hx-target="#cuerpo-lineas" hx-swap="beforeend" {
                "Añadir línea"
            }

            button.primario type="submit" { "Emitir factura" }
        }
    })
    .into_response()
}

/// El fragmento de una línea. Lo pide htmx y lo devuelve el mismo servidor.
pub async fn facturar_linea(State(st): State<Estado>, h: HeaderMap) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return *r;
    }
    // AQUÍ NO SIRVE `datos_o_aviso!`: esto devuelve un FRAGMENTO que htmx
    // inserta dentro de la tabla, no una pantalla entera. Meter una página de
    // aviso dentro de un `<tbody>` es peor que el defecto.
    //
    // Lo que sí se puede es no fingir. Con `unwrap_or_default` una base caída
    // devolvía una fila con el desplegable de artículos VACÍO, que el cajero lee
    // como «no hay artículos» — y el defecto es justo ése (directiva 30).
    //
    // EL AVISO SALE CON 200, Y NO ES DESCUIDO. La primera versión de este
    // arreglo devolvía 500 «porque es un error», y con eso el aviso NO LLEGABA A
    // LA PANTALLA: htmx 2.0.4 trae por defecto
    // `responseHandling: [… {code:"[45]..", swap:false, error:true}]` —leído del
    // artefacto vendorizado, `estaticos/htmx.min.js`, y este panel no define
    // ninguna configuración propia—, así que ante un 5xx **no intercambia
    // nada**. Al pulsar «Añadir línea» con la base caída no habría pasado
    // absolutamente nada: más mudo que el defecto que se venía a cerrar. Lo
    // encontró la revisión de seguridad del 2026-08-13.
    //
    // El 200 es además lo correcto de significado: la petición se atendió y su
    // respuesta ES este aviso. El fallo se cuenta en el contenido, que es lo que
    // el cajero puede leer, y en el registro por `mostrable`, que es donde va el
    // detalle. Un `hx-on::response-error` habría sido la otra salida, pero exige
    // que cada disparador lo recuerde.
    let articulos = match articulo::listar(&st.pool, "", false).await {
        Ok(v) => v,
        Err(e) => {
            return html! {
                tr.linea {
                    td colspan="6" {
                        span.aviso-error role="alert" {
                            "No se pudo leer la base: los artículos: " (mostrable(&e))
                        }
                    }
                }
            }
            .into_response();
        }
    };
    linea_de_factura(&articulos).into_response()
}

fn linea_de_factura(articulos: &[articulo::Articulo]) -> Markup {
    html! {
        tr.linea {
            td {
                // `data-sugerir` es lo que hace vivo al `data-precio`: sin él
                // el atributo viajaba en cada opción y no lo leía nadie.
                select name="linea_articulo"
                       data-sugerir="precio"
                       data-sugerir-a="input[name=linea_precio]" {
                    option value="" { "— elija —" }
                    @for a in articulos {
                        // El precio de lista viaja en el `option` para
                        // sugerirlo; el que manda sigue siendo el que se
                        // escriba, y por eso el comportamiento no pisa lo
                        // tecleado.
                        // En PESOS, no en centavos: el campo al que lo copia
                        // `sugerir-de-opcion` es el que teclea quien factura, y
                        // ése pasa por `intentar_de_pesos`. Emitirlo crudo haría
                        // la línea 100 veces cara.
                        option value=(a.id) data-precio=(a.precio_venta / 100) {
                            (a.codigo) " · " (a.descripcion)
                        }
                    }
                }
            }
            td.num { input type="number" name="linea_cantidad" min="1" step="1" value="1"; }
            td.num { input type="number" name="linea_precio" min="0" step="1" value="0"; }
            td.num { input type="number" name="linea_descuento" min="0" step="1" value="0"; }
            // LA TARIFA NO SE TECLEA. Es una propiedad del artículo, no de la
            // venta: la lee el servidor de su registro al emitir. La primera
            // versión de esta pantalla ponía un `value="19"` fijo para todos y
            // caía a 0 si el campo llegaba ilegible — un artículo exento se
            // facturaba al 19 %, y un campo en blanco emitía una factura SIN
            // IVA, con número de resolución quemado y CUFE calculado sobre la
            // base equivocada. Ninguna de las dos cosas daba error.
            td.num.tarifa { "según el artículo" }
        }
    }
}

pub async fn facturar_enviar(State(st): State<Estado>, h: HeaderMap, cuerpo: String) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Facturar) {
        return *r;
    }
    let s = sesion_de(&st, &h);
    let empresa = nombre_empresa(&st).await;
    let cs = campos(&cuerpo);

    let (Some(resolucion_id), Some(tercero_id)) = (
        entero(uno(&cs, "resolucion_id")),
        entero(uno(&cs, "tercero_id")),
    ) else {
        return formulario_factura(&st, &empresa, Some("Faltan la resolución o el cliente.")).await;
    };

    // Las cinco columnas se recorren EN PARALELO y por posición. Una fila a la
    // que le falte el artículo se DESCARTA entera —no se completa con ceros—, y
    // si al final no queda ninguna, se dice: una factura sin líneas emitida en
    // silencio quemaría un número de la resolución.
    let arts = todos(&cs, "linea_articulo");
    let cants = todos(&cs, "linea_cantidad");
    let precios = todos(&cs, "linea_precio");
    let descs = todos(&cs, "linea_descuento");

    let mut lineas = Vec::new();
    for (i, a) in arts.iter().enumerate() {
        let (Some(articulo_id), Some(cantidad)) = (entero(a), cants.get(i).and_then(|v| entero(v)))
        else {
            continue;
        };

        // LA TARIFA SALE DEL REGISTRO DEL ARTÍCULO, leído aquí. No viaja en el
        // formulario, así que no hay valor por omisión que equivocarse: si el
        // artículo no existe, la factura NO se emite —antes se rellenaba con 19
        // o con 0 y salía igual—.
        let art = match articulo::uno(&st.pool, articulo_id).await {
            Ok(a) => a,
            Err(e) => {
                return formulario_factura(
                    &st,
                    &empresa,
                    Some(&format!("El artículo {articulo_id} no se pudo leer: {}", mostrable(&e))),
                )
                .await;
            }
        };

        // Un precio ILEGIBLE no es cero (directiva 20): cero es un precio
        // legítimo —los obsequios existen— y por eso no se puede usar como
        // señal de «no se entendió». Se rechaza la factura entera.
        let precio = match precios.get(i) {
            Some(v) if v.trim().is_empty() => 0,
            Some(v) => match entero(v) {
                Some(n) => n,
                None => {
                    return formulario_factura(
                        &st,
                        &empresa,
                        Some(&format!("El precio de la línea {} no es un número.", i + 1)),
                    )
                    .await;
                }
            },
            None => 0,
        };

        // Mismo motivo que en el movimiento: lo tecleado se convierte con la
        // versión que puede decir que no.
        let (Some(cantidad), Some(precio_unitario), Some(descuento)) = (
            inventario_dominio::dinero::Cantidad::intentar_de_unidades(cantidad),
            inventario_dominio::dinero::Centavos::intentar_de_pesos(precio),
            inventario_dominio::dinero::Centavos::intentar_de_pesos(
                descs.get(i).and_then(|v| entero(v)).unwrap_or(0),
            ),
        ) else {
            return formulario_factura(
                &st,
                &empresa,
                Some(&format!("Los números de la línea {} son absurdos.", i + 1)),
            )
            .await;
        };

        lineas.push(inventario_dominio::factura::Linea {
            articulo_id,
            cantidad,
            precio_unitario,
            descuento,
            iva_porcentaje: art.iva_porcentaje,
            // El INC también sale del artículo. Hasta hoy iba SIEMPRE en cero
            // —el Angular tampoco lo mandaba—, así que ninguna factura de este
            // sistema había liquidado impuesto nacional al consumo, que el art.
            // 11 de la Res. 000165/2023 exige discriminar.
            inc_porcentaje: art.inc_porcentaje,
        });
    }

    if lineas.is_empty() {
        return formulario_factura(&st, &empresa, Some("No hay ninguna línea con artículo y cantidad.")).await;
    }

    let medio = uno(&cs, "medio_pago").trim().to_string();
    let p = inventario_datos::factura::Peticion {
        resolucion_id,
        tercero_id,
        vendedor_id: s.as_ref().map(|s| s.usuario_id),
        fecha: uno(&cs, "fecha").to_string(),
        hora: format!("{}-05:00", chrono::Utc::now().format("%H:%M:%S")),
        forma_pago: uno(&cs, "forma_pago").to_string(),
        medio_pago: if medio.is_empty() { None } else { Some(medio) },
        retencion: 0,
        observacion: uno(&cs, "observacion").to_string(),
        lineas,
    };

    match inventario_datos::factura::emitir(&st.pool, &p).await {
        Ok(e) => formulario_factura(
            &st,
            &empresa,
            Some(&format!("Factura {}{} emitida por {}.", e.prefijo, e.numero, pesos(e.total / 100))),
        )
        .await,
        Err(e) => formulario_factura(&st, &empresa, Some(&format!("No se emitió: {}", mostrable(&e)))).await,
    }
}

// ── Recaudar ──────────────────────────────────────────────────────────────────

pub async fn recaudar_pagina(
    State(st): State<Estado>,
    h: HeaderMap,
    Query(q): Query<SeleccionTercero>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return *r;
    }
    let empresa = nombre_empresa(&st).await;
    formulario_recibo(&st, &empresa, q.tercero_id, None).await
}

#[derive(Deserialize, Default)]
pub struct SeleccionTercero {
    #[serde(default)]
    tercero_id: Option<i64>,
}

async fn formulario_recibo(
    st: &Estado,
    empresa: &str,
    tercero_id: Option<i64>,
    dicho: Option<&str>,
) -> Response {
    let terceros = datos_o_aviso!(empresa, "los clientes", inventario_datos::tercero::listar(&st.pool, "", Some("cliente")).await);
    // Las facturas por cobrar solo se piden cuando hay cliente elegido: sin él
    // no hay nada que enseñar, y una tabla vacía se lee como «no debe nada».
    let pendientes = match tercero_id {
        Some(id) => datos_o_aviso!(empresa, "las facturas por cobrar", inventario_datos::factura::por_cobrar(&st.pool, id).await),
        None => Vec::new(),
    };

    envoltura("Recaudar", empresa, Some("/recaudar"), html! {
        h2 { "Recaudar" }
        @if let Some(m) = dicho { div.aviso-resultado role="status" { (m) } }

        // Elegir cliente recarga la página por htmx: la cartera que se enseña
        // es SIEMPRE la del cliente seleccionado, no una que quedó de antes.
        // SIN manejador en línea. La primera versión llevaba
        // `onchange="this.form.submit()"`, y con `script-src 'self'` el navegador
        // NO lo ejecuta: elegir un cliente no hacía absolutamente nada, y el
        // `<noscript>` tampoco salvaba —JavaScript está activo, lo bloqueado era
        // el manejador—. El arreglo NO es abrir la CSP con `unsafe-inline`, que
        // devolvería el XSS que esa cabecera acaba de cerrar: es un botón normal,
        // que además funciona sin JavaScript (mejora progresiva).
        form.buscador method="get" action="/recaudar" {
            label { "Cliente"
                select name="tercero_id" {
                    option value="" { "— elija —" }
                    @for t in &terceros {
                        option value=(t.id) selected[tercero_id == Some(t.id)] {
                            (t.nit_completo) " · " (t.nombre)
                        }
                    }
                } }
            button.secundario type="submit" { "Ver cartera" }
        }

        @if let Some(id) = tercero_id {
            h3 { "Facturas pendientes" }
            @if pendientes.is_empty() {
                p.vacio { "Este cliente no tiene facturas a crédito pendientes." }
            } @else {
                table.datos {
                    thead { tr {
                        th.num { "Factura" } th.num { "Total" } th.num { "Abonado" }
                        th.num { "Notas" } th.num { "Debe" }
                    } }
                    tbody {
                        @for f in &pendientes {
                            tr {
                                td.num { (f.factura_id) }
                                td.num { (pesos(f.total.0 / 100)) }
                                td.num { (pesos(f.abonado.0 / 100)) }
                                td.num { (pesos(f.notas.0 / 100)) }
                                td.num { (pesos(f.saldo().0 / 100)) }
                            }
                        }
                    }
                }

                form.ficha method="post" action="/recaudar" {
                    input type="hidden" name="tercero_id" value=(id);
                    div.rejilla {
                        label { "Fecha"
                            input type="date" name="fecha" value=(crate::rutas::hoy()) required; }
                        label { "Observación"
                            input name="observacion" maxlength="200"; }
                    }
                    h3 { "Pagos" }
                    table.datos #pagos {
                        thead { tr { th { "Medio" } th { "Referencia" } th.num { "Valor" } } }
                        tbody #cuerpo-pagos { (linea_de_pago()) }
                    }
                    button.secundario type="button"
                           hx-get="/recaudar/pago" hx-target="#cuerpo-pagos" hx-swap="beforeend" {
                        "Añadir pago"
                    }
                    p.nota {
                        "Sin aplicaciones explícitas, el recibo se reparte de la factura más \
                         vieja a la más nueva."
                    }
                    button.primario type="submit" { "Registrar recibo" }
                }
            }
        }
    })
    .into_response()
}

pub async fn recaudar_pago(State(st): State<Estado>, h: HeaderMap) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return *r;
    }
    linea_de_pago().into_response()
}

fn linea_de_pago() -> Markup {
    html! {
        tr.linea {
            td {
                // Los medios salen del DOMINIO, no de una lista escrita aquí:
                // esta ofrecía «tarjeta», que `recibo::registrar` rechaza, así
                // que cobrar con tarjeta desde el panel era imposible.
                select name="pago_medio" {
                    @for (valor, etiqueta) in inventario_dominio::factura::MEDIOS_DE_PAGO {
                        option value=(valor) { (etiqueta) }
                    }
                }
            }
            td { input name="pago_referencia" maxlength="40"; }
            td.num { input type="number" name="pago_valor" min="1" step="1" value="0"; }
        }
    }
}

pub async fn recaudar_enviar(State(st): State<Estado>, h: HeaderMap, cuerpo: String) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Recaudar) {
        return *r;
    }
    let s = sesion_de(&st, &h);
    let empresa = nombre_empresa(&st).await;
    let cs = campos(&cuerpo);

    let Some(tercero_id) = entero(uno(&cs, "tercero_id")) else {
        return formulario_recibo(&st, &empresa, None, Some("Falta el cliente.")).await;
    };

    let medios = todos(&cs, "pago_medio");
    let refs = todos(&cs, "pago_referencia");
    let valores = todos(&cs, "pago_valor");
    let mut pagos = Vec::new();
    for (i, m) in medios.iter().enumerate() {
        // Un pago de cero no es un pago: se descarta en vez de guardar un
        // renglón que no mueve nada y ensucia el recibo.
        let Some(valor) = valores.get(i).and_then(|v| entero(v)).filter(|v| *v > 0) else {
            continue;
        };
        // El ÚLTIMO `×100` tecleado que quedaba crudo. Lo señaló la revisión
        // independiente: el punto 7 cerró la factura y el movimiento y se dejó
        // el recibo. Desborda —y con `overflow-checks = true` eso es pánico en
        // los dos perfiles—, y el caso que sobrevive a las redes de
        // `repartir` es una suma de pagos que desborda
        // pequeño, dejando `recibo.total` incoherente con `SUM(recibo_pago.valor)`.
        let Some(centavos) = inventario_dominio::dinero::Centavos::intentar_de_pesos(valor) else {
            return formulario_recibo(
                &st,
                &empresa,
                Some(tercero_id),
                Some("Uno de los pagos es un importe absurdo."),
            )
            .await;
        };
        pagos.push(inventario_datos::recibo::Pago {
            medio: (*m).to_string(),
            banco_id: None,
            referencia: refs.get(i).map(|r| r.to_string()).unwrap_or_default(),
            vence: None,
            valor: centavos.0,
        });
    }

    if pagos.is_empty() {
        return formulario_recibo(&st, &empresa, Some(tercero_id), Some("No hay ningún pago con valor.")).await;
    }

    let p = inventario_datos::recibo::Peticion {
        tercero_id,
        fecha: uno(&cs, "fecha").to_string(),
        observacion: uno(&cs, "observacion").to_string(),
        pagos,
        aplicaciones: Vec::new(),
    };

    match inventario_datos::recibo::registrar(&st.pool, &p, s.map(|s| s.usuario_id)).await {
        Ok(r) => {
            // El SOBRANTE se dice. `recibo::registrar` lo devuelve calculado y
            // `/api` lo publica, pero el panel solo anunciaba el número: era
            // efectivo entrando en caja sin destino visible, y quien opera se
            // enteraba al cuadrar. Cuando es cero no se nombra, para no enseñar
            // un renglón que no dice nada.
            let aviso = if r.sobrante > 0 {
                format!(
                    "Recibo {} registrado. Se aplicaron ${} y quedan ${} SIN APLICAR: \
                     el cliente no debía tanto.",
                    r.numero,
                    pesos(r.aplicado),
                    pesos(r.sobrante)
                )
            } else {
                format!("Recibo {} registrado por ${}.", r.numero, pesos(r.total))
            };
            formulario_recibo(&st, &empresa, Some(tercero_id), Some(&aviso)).await
        }
        Err(e) => formulario_recibo(&st, &empresa, Some(tercero_id), Some(&format!("No se registró: {}", mostrable(&e)))).await,
    }
}

// ── Configuración ─────────────────────────────────────────────────────────────

pub async fn configuracion_pagina(State(st): State<Estado>, h: HeaderMap) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return *r;
    }
    let empresa_nombre = nombre_empresa(&st).await;
    pantalla_configuracion(&st, &empresa_nombre, None).await
}

async fn pantalla_configuracion(st: &Estado, empresa_nombre: &str, dicho: Option<&str>) -> Response {
    let e = catalogo::empresa(&st.pool).await.ok().flatten();
    let usuarios = datos_o_aviso!(empresa_nombre, "los usuarios", usuario::listar(&st.pool).await);
    let conceptos = datos_o_aviso!(empresa_nombre, "los conceptos", catalogo::conceptos(&st.pool).await);
    let resoluciones = datos_o_aviso!(
        empresa_nombre,
        "las resoluciones de facturación",
        inventario_datos::resolucion::listar(&st.pool, &crate::rutas::hoy()).await
    );

    envoltura("Configuración", empresa_nombre, Some("/configuracion"), html! {
        h2 { "Configuración" }
        @if let Some(m) = dicho { div.aviso-resultado role="status" { (m) } }

        h3 { "Empresa" }
        // El formulario se dibuja HAYA o NO empresa, y esa es la corrección:
        // antes, sin empresa configurada, el panel decía «la instalación no
        // está configurada todavía» y no ofrecía ninguna forma de configurarla.
        // O sea que lo PRIMERO que el instalador manda hacer —«falta configurar
        // la empresa (nombre y NIT); hasta entonces la facturación no debe
        // usarse»— era lo único que no se podía hacer desde el producto: había
        // que salir a `PUT /api/empresa` con un cliente HTTP.
        //
        // Un solo formulario para las dos veces, no dos caminos (directiva 1):
        // `guardar_empresa` ya hace UPSERT, así que la primera vez y la
        // corrección son la misma operación.
        @let primera_vez = e.is_none();
        @let v = |leer: &dyn Fn(&catalogo::Empresa) -> String| {
            e.as_ref().map(leer).unwrap_or_default()
        };
        @if primera_vez {
            p.aviso {
                b { "Esta instalación todavía no tiene empresa configurada." }
                " La facturación no debe usarse hasta que se llenen al menos el nombre y el NIT."
            }
        }
        form.ficha method="post" action="/configuracion/empresa" {
            div.rejilla {
                label { "Nombre" input name="nombre" value=(v(&|e| e.nombre.clone())) required; }
                label { "NIT" input name="nit" value=(v(&|e| e.nit.clone())) required; }
                // El DV no se teclea: lo CALCULA el dominio desde el NIT. Un
                // campo editable invita a escribir uno que no corresponde, y la
                // DIAN rechaza el documento por eso.
                label { "DV (calculado)"
                    input value=(e.as_ref().map(|e| e.dv.to_string()).unwrap_or_else(|| "—".into())) disabled; }
                label { "Dirección" input name="direccion" value=(v(&|e| e.direccion.clone())); }
                label { "Teléfono" input name="telefono" value=(v(&|e| e.telefono.clone())); }
                label { "Ambiente DIAN"
                    select name="ambiente_dian" {
                        // «Pruebas» es lo que sale marcado en una instalación
                        // nueva, y no por comodidad: una instalación a medio
                        // configurar que arranque en «producción» manda
                        // documentos de verdad a la DIAN.
                        option value="pruebas" selected[v(&|e| e.ambiente_dian.clone()) != "produccion"] { "Pruebas" }
                        option value="produccion" selected[v(&|e| e.ambiente_dian.clone()) == "produccion"] { "Producción" }
                    } }
                label { "Fórmula de costeo"
                    select name="formula_costeo" {
                        option value="promedio" selected[v(&|e| e.formula_costeo.clone()) != "peps"] { "Promedio ponderado" }
                        option value="peps" selected[v(&|e| e.formula_costeo.clone()) == "peps"] { "PEPS" }
                    } }
                label.casilla {
                    input type="checkbox" name="permite_negativo" value="true"
                          checked[e.as_ref().is_some_and(|e| e.permite_negativo)];
                    " Permitir existencia negativa"
                }
            }
            // El PIN y el ID de software NO se dibujan, ni siquiera en un campo
            // de contraseña: lo que la pantalla enseña lo ve cualquiera que mire
            // por encima del hombro, y esto va en un mostrador. Se cambian por
            // su propio formulario.
            p.nota {
                "El PIN y el identificador del software no se muestran aquí. "
                @if v(&|e| e.pin_software.clone()).is_empty() { b { "Hoy NO hay PIN configurado." } }
                @else { "Hay uno configurado." }
                " Se cambian en el formulario de abajo."
            }
            button.primario type="submit" { "Guardar empresa" }
        }

        // El formulario que el aviso de arriba prometía y que no existía: hasta
        // hoy el PIN solo se podía poner con un cliente HTTP contra
        // `PUT /api/empresa`. Sin PIN, `calcular_cude` no puede firmar las notas
        // crédito y débito, y eso no se descubre aquí sino cuando la DIAN
        // rechaza el documento.
        h3 { "Credenciales del software (DIAN)" }
        @if primera_vez {
            p.nota { "Primero hay que guardar la empresa; después se pueden cargar las credenciales." }
        } @else {
            form.ficha method="post" action="/configuracion/software" {
                div.rejilla {
                    // `type="password"`: esto se teclea en un mostrador, y lo
                    // que la pantalla enseña lo ve quien pase por detrás.
                    label { "PIN del software"
                        input type="password" name="pin_software" autocomplete="off"
                              placeholder=(if v(&|e| e.pin_software.clone()).is_empty() { "sin configurar" } else { "hay uno configurado" }); }
                    label { "Identificador del software"
                        input name="id_software" autocomplete="off"
                              placeholder=(if v(&|e| e.id_software.clone()).is_empty() { "sin configurar" } else { "hay uno configurado" }); }
                }
                p.nota {
                    "Dejar un campo en blanco CONSERVA el valor que ya estaba: "
                    "mandar vacío lo borraría, y una instalación sin PIN emite notas sin CUDE."
                }
                button.primario type="submit" { "Guardar credenciales" }
            }
        }

        h3 { "Usuarios" }
        table.datos {
            thead { tr { th { "Código" } th { "Nombre" } th { "Rol" } th { "Estado" } } }
            tbody {
                @for u in &usuarios {
                    tr.inactivo[!u.activo] {
                        td { code { (u.codigo) } }
                        td { (u.nombre) }
                        td { (u.rol) }
                        td { @if u.activo { "Activo" } @else { "Inactivo" } }
                    }
                }
            }
        }
        form.ficha method="post" action="/configuracion/usuario" {
            div.rejilla {
                label { "Código" input name="codigo" required maxlength="20"; }
                label { "Nombre" input name="nombre" required; }
                label { "Rol"
                    select name="rol" required {
                        option value="admin" { "Administrador" }
                        option value="cajero" { "Cajero" }
                        option value="bodega" { "Bodega" }
                        option value="consulta" { "Consulta" }
                    } }
                label { "Clave" input type="password" name="clave" required minlength="8"; }
            }
            button.primario type="submit" { "Crear usuario" }
        }

        h3 { "Resoluciones de facturación" }
        div.tabla-ancha {
            table.datos {
                thead { tr {
                    th { "Nombre" } th { "Prefijo" } th { "Número" } th.num { "Desde" }
                    th.num { "Hasta" } th.num { "Quedan" } th { "Vence" } th { "Estado" }
                } }
                tbody {
                    @for r in &resoluciones {
                        tr {
                            td { (r.nombre) }
                            td { code { (r.prefijo) } }
                            td { (r.numero) }
                            td.num { (r.desde) }
                            td.num { (r.hasta) }
                            // Quedar sin números es lo que para la caja: se
                            // marca cuando ya no quedan, no antes.
                            td.num.alerta[r.quedan == 0] { (r.quedan) }
                            td { (r.vence.clone().unwrap_or_default()) }
                            // El IMPEDIMENTO se enseña tal cual lo dice el
                            // dominio. «Inactiva» y «vencida» y «agotada» son
                            // tres problemas distintos con tres arreglos
                            // distintos: colapsarlos en «no usable» obliga a
                            // adivinar cuál es.
                            td { (r.impedimento.clone().unwrap_or_else(|| "Usable".into())) }
                        }
                    }
                }
            }
        }

        // Sin esto la tabla de arriba es un callejón sin salida: el panel avisa
        // en rojo de que la resolución se agotó o venció y no ofrece ninguna
        // forma de registrar la que la sustituye. La DIAN autoriza por 6, 12,
        // 18 o 24 meses, y el rango y la vigencia **se agotan por separado**
        // (Res. 000165/2023), así que hay dos fechas y un rango, no uno solo.
        form.ficha method="post" action="/configuracion/resolucion" {
            div.rejilla {
                label { "Nombre" input name="nombre" required; }
                label { "Número de resolución" input name="numero"; }
                label { "Prefijo" input name="prefijo" maxlength="4"; }
                label { "Fecha de la resolución" input type="date" name="fecha"; }
                label { "Rige desde" input type="date" name="rige_desde"; }
                label { "Vence" input type="date" name="vence"; }
                label { "Desde (número)" input type="number" name="desde" min="1" required; }
                label { "Hasta (número)" input type="number" name="hasta" min="1" required; }
                // Sin clave técnica las facturas de esta resolución salen sin
                // CUFE. No es obligatoria en el formulario porque una
                // resolución de contingencia puede no traerla, pero se dice.
                label { "Clave técnica" input name="clave_tecnica" autocomplete="off"; }
            }
            p.nota { "Sin clave técnica, las facturas de esta resolución salen SIN CUFE." }
            button.primario type="submit" { "Registrar resolución" }
        }

        h3 { "Conceptos de movimiento" }
        // El efecto tributario se ENSEÑA y se puede CORREGIR aquí, y las dos
        // cosas son la misma decisión: el migrador deja todo concepto de salida
        // heredado en «no afecta impuestos» —el corpus no traía el dato— y lo
        // marca para revisión humana. Si la pantalla no lo muestra, esa
        // revisión no se puede hacer; si no lo deja cambiar, no se puede
        // ejecutar. Son 161 movimientos de obsequio, consumo interno y retiro
        // de socios en el sistema anterior, todos sin un peso de IVA.
        @let neutros = conceptos.iter().filter(|c| c.signo == "salida"
            && c.efecto_tributario == "no_afecta_impuestos" && c.activo).count();
        @if neutros > 0 {
            p.aviso {
                b { (neutros) " conceptos de SALIDA no causan ningún impuesto." }
                " Si alguno es obsequio, consumo interno o retiro de socios, causa IVA sobre el "
                "valor comercial (ET art. 421 lit. b) y hay que corregirlo aquí antes de operar."
            }
        }
        div.tabla-ancha {
            table.datos {
                thead { tr {
                    th { "Código" } th { "Nombre" } th { "Signo" } th { "Clase" }
                    th { "Efecto tributario" } th { "Exige tercero" } th { "Estado" }
                } }
                tbody {
                    @for c in &conceptos {
                        tr.inactivo[!c.activo] {
                            td { code { (c.codigo) } }
                            td { (c.nombre) }
                            td { (c.signo) }
                            td { (c.clase) }
                            td.alerta[c.signo == "salida" && c.efecto_tributario == "no_afecta_impuestos"] {
                                (etiqueta_efecto(&c.efecto_tributario))
                            }
                            td { @if c.exige_tercero { "Sí" } @else { "No" } }
                            td { @if c.activo { "Activo" } @else { "Inactivo" } }
                        }
                    }
                }
            }
        }

        h4 { "Corregir el efecto tributario de un concepto" }
        form.ficha method="post" action="/configuracion/concepto/efecto" {
            div.rejilla {
                label { "Concepto"
                    select name="concepto_id" required {
                        option value="" { "—" }
                        @for c in &conceptos {
                            option value=(c.id) {
                                (c.codigo) " · " (c.nombre)
                                " (" (etiqueta_efecto(&c.efecto_tributario)) ")"
                            }
                        }
                    } }
                label { "Efecto tributario" (selector_de_efecto("efecto_tributario", None)) }
            }
            p.nota {
                "Corrige de aquí en adelante. Los movimientos ya registrados guardaron su "
                "impuesto al escribirse y no se reescriben: para arreglar el pasado hay que "
                "reversar el movimiento."
            }
            button.primario type="submit" { "Corregir" }
        }

        h4 { "Nuevo concepto" }
        form.ficha method="post" action="/configuracion/concepto" {
            div.rejilla {
                label { "Código" input name="codigo" required maxlength="10"; }
                label { "Nombre" input name="nombre" required; }
                label { "Signo"
                    select name="signo" required {
                        option value="entrada" { "Entrada" }
                        option value="salida" { "Salida" }
                    } }
                label { "Clase"
                    select name="clase" required {
                        @for c in ["compra", "venta", "devolucion_venta", "devolucion_compra",
                                   "ajuste", "traslado", "inicial", "consumo"] {
                            option value=(c) { (c) }
                        }
                    } }
                // NO hay opción preseleccionada, y es la regla 7 del repositorio
                // llevada a la pantalla: `efecto_tributario` no tiene valor por
                // defecto en el esquema justamente para que un concepto nuevo
                // de obsequio no nazca sin IVA por omisión. Un `selected` aquí
                // reabriría esa brecha desde el HTML.
                label { "Efecto tributario" (selector_de_efecto("efecto_tributario", None)) }
                label.casilla {
                    input type="checkbox" name="exige_tercero" value="true";
                    " Exige tercero"
                }
            }
            button.primario type="submit" { "Crear concepto" }
        }
    })
    .into_response()
}

/// El efecto tributario en palabras, con su norma. `retiro_gravado` a secas no
/// le dice nada a quien tiene que decidir si un obsequio lo es.
fn etiqueta_efecto(e: &str) -> &'static str {
    match e {
        "venta" => "Venta",
        "retiro_gravado" => "Retiro gravado — causa IVA (ET art. 421 lit. b)",
        "faltante_art64" => "Faltante — consume el tope del 3 % (ET art. 64)",
        "baja_con_acta" => "Baja con acta (ET art. 64)",
        "no_afecta_impuestos" => "No afecta impuestos",
        // La lista del esquema es cerrada; si aparece otro valor se DICE, no se
        // dibuja como si fuera uno de los conocidos (directiva 20).
        _ => "DESCONOCIDO — revisar",
    }
}

/// El selector de efecto tributario, sin opción marcada por defecto.
///
/// `elegido` existe para reusarlo donde sí haya un valor previo; hoy nadie lo
/// pasa, y esa es la decisión: al crear o corregir, quien decide es la persona.
fn selector_de_efecto(nombre: &str, elegido: Option<&str>) -> Markup {
    html! {
        select name=(nombre) required {
            option value="" selected[elegido.is_none()] { "— elija —" }
            @for v in ["venta", "retiro_gravado", "faltante_art64", "baja_con_acta",
                       "no_afecta_impuestos"] {
                option value=(v) selected[elegido == Some(v)] { (etiqueta_efecto(v)) }
            }
        }
    }
}

pub async fn configuracion_empresa(State(st): State<Estado>, h: HeaderMap, cuerpo: String) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return *r;
    }
    let empresa_nombre = nombre_empresa(&st).await;
    let cs = campos(&cuerpo);

    // Se arma la MISMA entrada que recibe `PUT /api/empresa` y se llama a la
    // MISMA función: la normalización del NIT, el dígito de verificación y la
    // validación de la fórmula de costeo viven en un solo sitio (directiva 1).
    //
    // El PIN y el identificador del software no viajan por este formulario, así
    // que se conservan los que había: mandar cadena vacía los BORRARÍA, y sin
    // PIN las facturas salen sin CUDE.
    let previa = catalogo::empresa(&st.pool).await.ok().flatten();
    let e = crate::rutas::EmpresaEntrante {
        nombre: uno(&cs, "nombre").to_string(),
        nit: uno(&cs, "nit").to_string(),
        direccion: uno(&cs, "direccion").to_string(),
        telefono: uno(&cs, "telefono").to_string(),
        // Una casilla sin marcar NO llega en el cuerpo: su ausencia significa
        // «false». Eso lo define el navegador, así que aquí sí es una lectura
        // correcta y no un relleno por omisión.
        permite_negativo: !uno(&cs, "permite_negativo").is_empty(),
        formula_costeo: uno(&cs, "formula_costeo").to_string(),
        ambiente_dian: uno(&cs, "ambiente_dian").to_string(),
        pin_software: previa.as_ref().map(|v| v.pin_software.clone()).unwrap_or_default(),
        id_software: previa.as_ref().map(|v| v.id_software.clone()).unwrap_or_default(),
    };

    match crate::rutas::guardar_empresa(&st, &e).await {
        Ok(nit) => {
            let nombre = e.nombre.clone();
            pantalla_configuracion(&st, &nombre, Some(&format!("Empresa guardada. NIT {nit}."))).await
        }
        Err(err) => pantalla_configuracion(&st, &empresa_nombre, Some(&format!("No se guardó: {}", mostrable(&err)))).await,
    }
}

pub async fn configuracion_usuario(State(st): State<Estado>, h: HeaderMap, cuerpo: String) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return *r;
    }
    let empresa_nombre = nombre_empresa(&st).await;
    let cs = campos(&cuerpo);
    let clave = uno(&cs, "clave");

    if clave.len() < 8 {
        return pantalla_configuracion(&st, &empresa_nombre, Some("La clave debe tener al menos 8 caracteres.")).await;
    }
    let Some(hash) = crate::claves::hashear(clave) else {
        return pantalla_configuracion(&st, &empresa_nombre, Some("No se pudo cifrar la clave.")).await;
    };
    match usuario::crear(&st.pool, uno(&cs, "codigo"), uno(&cs, "nombre"), &hash, uno(&cs, "rol")).await {
        Ok(_) => pantalla_configuracion(&st, &empresa_nombre, Some("Usuario creado.")).await,
        Err(e) => pantalla_configuracion(&st, &empresa_nombre, Some(&format!("No se creó: {}", mostrable(&e)))).await,
    }
}

/// Las credenciales de facturación electrónica, por su propio formulario.
///
/// **Un campo en blanco CONSERVA lo que había.** No es tolerancia: mandar la
/// cadena vacía las borraría, y una instalación sin PIN emite notas sin CUDE
/// —lo que no se descubre aquí, sino cuando la DIAN rechaza el documento—. Como
/// el formulario no puede mostrar el valor actual (es una credencial), la única
/// lectura segura del vacío es «no lo toques».
pub async fn configuracion_software(State(st): State<Estado>, h: HeaderMap, cuerpo: String) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return *r;
    }
    let empresa_nombre = nombre_empresa(&st).await;
    let cs = campos(&cuerpo);

    let Some(previa) = catalogo::empresa(&st.pool).await.ok().flatten() else {
        return pantalla_configuracion(
            &st,
            &empresa_nombre,
            Some("No hay empresa configurada todavía: guárdela primero."),
        )
        .await;
    };

    let pin = uno(&cs, "pin_software").trim();
    let id_sw = uno(&cs, "id_software").trim();
    let e = crate::rutas::EmpresaEntrante {
        nombre: previa.nombre.clone(),
        nit: previa.nit.clone(),
        direccion: previa.direccion.clone(),
        telefono: previa.telefono.clone(),
        permite_negativo: previa.permite_negativo,
        formula_costeo: previa.formula_costeo.clone(),
        ambiente_dian: previa.ambiente_dian.clone(),
        pin_software: if pin.is_empty() { previa.pin_software.clone() } else { pin.to_string() },
        id_software: if id_sw.is_empty() { previa.id_software.clone() } else { id_sw.to_string() },
    };

    match crate::rutas::guardar_empresa(&st, &e).await {
        // El dicho NO repite el PIN, ni recortado: se dice qué se cambió, no
        // qué vale. Un eco en pantalla lo deja legible a quien pase por detrás
        // y en el historial del navegador.
        Ok(_) => {
            let que = match (pin.is_empty(), id_sw.is_empty()) {
                (true, true) => "No se cambió nada: los dos campos venían en blanco.",
                (false, true) => "PIN del software actualizado.",
                (true, false) => "Identificador del software actualizado.",
                (false, false) => "PIN e identificador del software actualizados.",
            };
            pantalla_configuracion(&st, &empresa_nombre, Some(que)).await
        }
        Err(err) => {
            pantalla_configuracion(&st, &empresa_nombre, Some(&format!("No se guardó: {}", mostrable(&err)))).await
        }
    }
}

/// Registra una resolución de facturación.
pub async fn configuracion_resolucion(State(st): State<Estado>, h: HeaderMap, cuerpo: String) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return *r;
    }
    let empresa_nombre = nombre_empresa(&st).await;
    let cs = campos(&cuerpo);

    // El rango NO se rellena con cero si viene ilegible: un cero aquí es una
    // resolución que arranca donde no debe y quema números de la DIAN.
    let (Some(desde), Some(hasta)) = (entero(uno(&cs, "desde")), entero(uno(&cs, "hasta"))) else {
        return pantalla_configuracion(
            &st,
            &empresa_nombre,
            Some("El rango «desde» y «hasta» tiene que ser dos números enteros."),
        )
        .await;
    };

    let opcional = |c: &str| {
        let v = uno(&cs, c).trim().to_string();
        (!v.is_empty()).then_some(v)
    };
    let n = inventario_datos::resolucion::Nueva {
        nombre: uno(&cs, "nombre").to_string(),
        prefijo: uno(&cs, "prefijo").to_string(),
        numero: uno(&cs, "numero").to_string(),
        fecha: opcional("fecha"),
        rige_desde: opcional("rige_desde"),
        vence: opcional("vence"),
        clave_tecnica: opcional("clave_tecnica"),
        desde,
        hasta,
    };
    match inventario_datos::resolucion::crear(&st.pool, &n).await {
        Ok(_) => {
            let aviso = if n.clave_tecnica.is_none() {
                "Resolución registrada. AVISO: sin clave técnica, sus facturas salen sin CUFE."
            } else {
                "Resolución registrada."
            };
            pantalla_configuracion(&st, &empresa_nombre, Some(aviso)).await
        }
        Err(e) => pantalla_configuracion(&st, &empresa_nombre, Some(&format!("No se registró: {}", mostrable(&e)))).await,
    }
}

/// Crea un concepto de kardex.
pub async fn configuracion_concepto(State(st): State<Estado>, h: HeaderMap, cuerpo: String) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return *r;
    }
    let empresa_nombre = nombre_empresa(&st).await;
    let cs = campos(&cuerpo);

    // El efecto vacío NO se convierte en «no afecta impuestos». Es la regla 7
    // del repositorio: el esquema no le pone valor por defecto justamente para
    // que un obsequio nuevo no nazca sin IVA por omisión, y la pantalla no
    // puede ser la puerta de atrás de esa decisión.
    let efecto = uno(&cs, "efecto_tributario").trim();
    if efecto.is_empty() {
        return pantalla_configuracion(
            &st,
            &empresa_nombre,
            Some("Hay que elegir el efecto tributario: no tiene valor por defecto a propósito."),
        )
        .await;
    }

    match catalogo::crear_concepto(
        &st.pool,
        uno(&cs, "codigo"),
        uno(&cs, "nombre"),
        uno(&cs, "signo"),
        uno(&cs, "clase"),
        efecto,
        !uno(&cs, "exige_tercero").is_empty(),
    )
    .await
    {
        Ok(_) => pantalla_configuracion(&st, &empresa_nombre, Some("Concepto creado.")).await,
        Err(e) => pantalla_configuracion(&st, &empresa_nombre, Some(&format!("No se creó: {}", mostrable(&e)))).await,
    }
}

/// Corrige el efecto tributario de un concepto que ya existe.
///
/// Es la mitad de pantalla de la revisión que el migrador pide en su informe.
pub async fn configuracion_concepto_efecto(State(st): State<Estado>, h: HeaderMap, cuerpo: String) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return *r;
    }
    let empresa_nombre = nombre_empresa(&st).await;
    let cs = campos(&cuerpo);

    let Some(id) = entero(uno(&cs, "concepto_id")) else {
        return pantalla_configuracion(&st, &empresa_nombre, Some("Elija el concepto que va a corregir.")).await;
    };
    let efecto = uno(&cs, "efecto_tributario").trim();
    if efecto.is_empty() {
        return pantalla_configuracion(&st, &empresa_nombre, Some("Elija el efecto tributario.")).await;
    }

    match catalogo::actualizar_efecto_tributario(&st.pool, id, efecto).await {
        // El número de movimientos ya escritos se DICE. La corrección vale de
        // aquí en adelante, y quien la hace tiene que enterarse en el momento
        // de que llegó después de que el concepto ya moviera kardex — no al
        // cerrar el bimestre.
        Ok(0) => pantalla_configuracion(&st, &empresa_nombre, Some("Concepto corregido. No tenía ningún movimiento registrado.")).await,
        Ok(ya) => {
            let m = format!(
                "Concepto corregido. AVISO: ya tenía {ya} movimiento(s) registrado(s), \
                 que conservan el impuesto con que se escribieron. La corrección vale \
                 de aquí en adelante; para arreglar el pasado hay que reversarlos."
            );
            pantalla_configuracion(&st, &empresa_nombre, Some(&m)).await
        }
        Err(e) => pantalla_configuracion(&st, &empresa_nombre, Some(&format!("No se corrigió: {}", mostrable(&e)))).await,
    }
}

/// Corre el corte de deterioro por VNR a una fecha (NIC 2.28–33).
///
/// **Escribe**, así que es un POST y no un efecto de mirar la pantalla. Pide
/// `Configurar` y no `Consultar`: medir el inventario al menor entre costo y
/// VNR mueve el valor en libros, y eso no lo hace quien solo consulta.
pub async fn informes_deterioro(State(st): State<Estado>, h: HeaderMap, cuerpo: String) -> Response {
    let s = match exigir(&st, &h, usuario::Permiso::Configurar) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let cs = campos(&cuerpo);
    let fecha = uno(&cs, "fecha").to_string();
    let q = RangoInformes {
        corte: Some(fecha.clone()),
        ..RangoInformes::default()
    };
    match inventario_datos::deterioro::correr_corte(&st.pool, &fecha, Some(s.usuario_id)).await {
        Ok(c) => {
            // Lo que NO se pudo medir se dice por su nombre y en el momento: un
            // corte que omitiera los artículos sin precio estimado diría que el
            // inventario está medido cuando no lo está (directiva 20).
            let m = if c.sin_estimacion.is_empty() {
                format!(
                    "Corte al {fecha} corrido: {} partidas medidas, {} de rebajas y {} de reversiones.",
                    c.revelacion.partidas_medidas,
                    pesos(c.revelacion.rebajas_del_periodo),
                    pesos(c.revelacion.reversiones_del_periodo)
                )
            } else {
                format!(
                    "Corte al {fecha} corrido: {} partidas medidas. AVISO: {} artículo(s) con \
                     existencia NO se pudieron medir por falta de precio de venta estimado ({}).",
                    c.revelacion.partidas_medidas,
                    c.sin_estimacion.len(),
                    c.sin_estimacion.join(", ")
                )
            };
            pantalla_informes(&st, &RangoInformes { dicho: Some(m), ..q }).await
        }
        Err(e) => {
            let m = format!("No se pudo correr el corte: {}", mostrable(&e));
            pantalla_informes(&st, &RangoInformes { dicho: Some(m), ..q }).await
        }
    }
}

/// Levanta el acta de baja del art. 64 sobre las salidas que se marcaron.
pub async fn informes_acta(State(st): State<Estado>, h: HeaderMap, cuerpo: String) -> Response {
    let s = match exigir(&st, &h, usuario::Permiso::Configurar) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let cs = campos(&cuerpo);
    let volver = |dicho: String| RangoInformes {
        desde: Some(uno(&cs, "desde").to_string()),
        hasta: Some(uno(&cs, "hasta").to_string()),
        dicho: Some(dicho),
        ..RangoInformes::default()
    };

    // Una casilla sin marcar NO llega en el cuerpo, así que los movimientos que
    // llegan son exactamente los elegidos. Las justificaciones sí llegan TODAS
    // —un `input` de texto viaja aunque su fila no esté marcada—, de modo que
    // casarlas por posición sería casar la fila 1 marcada con la justificación
    // de la fila 1 de la TABLA, que puede ser otra. Por eso se leen las dos
    // listas y se cruza por el índice de la fila, no por el de los marcados.
    let marcados: Vec<i64> = todos(&cs, "movimiento").iter().filter_map(|v| entero(v)).collect();
    if marcados.is_empty() {
        return pantalla_informes(&st, &volver("Marque al menos una baja para el acta.".into())).await;
    }

    // Se releen las bajas pendientes para saber a qué artículo pertenece cada
    // movimiento y cuántas unidades quedan: pedirle esos datos al formulario
    // sería dejar que el navegador declare el costo fiscal.
    let desde = uno(&cs, "desde").to_string();
    let hasta = uno(&cs, "hasta").to_string();
    let pendientes = match inventario_datos::tributario::bajas_sin_acta(&st.pool, &desde, &hasta).await {
        Ok(p) => p,
        Err(e) => {
            return pantalla_informes(&st, &volver(format!("No se pudo leer: {}", mostrable(&e)))).await;
        }
    };

    // La justificación de cada fila, casada por la POSICIÓN de la fila en la
    // tabla que se dibujó, que es la misma que la de `pendientes`.
    let justificaciones = todos(&cs, "justificacion");
    let mut renglones = Vec::new();
    for (i, b) in pendientes.iter().enumerate() {
        if !marcados.contains(&b.movimiento_id) {
            continue;
        }
        renglones.push(inventario_datos::tributario::PeticionRenglon {
            articulo_id: b.articulo_id,
            cantidad: b.pendiente,
            justificacion: justificaciones.get(i).copied().unwrap_or("").trim().to_string(),
            movimiento_id: b.movimiento_id,
        });
    }
    if renglones.is_empty() {
        return pantalla_informes(&st, &volver("Las bajas marcadas ya no están pendientes.".into())).await;
    }

    // **UN IMPORTE ILEGIBLE NO ES CERO** (directiva 20), y aquí costó dos
    // intentos. El commit anterior decía haber quitado el `unwrap_or(0)` y lo
    // único que hizo fue meterlo DENTRO de `intentar_de_pesos`: la rama de error
    // solo disparaba si el `×100` desbordaba, así que «1.000» tecleado con punto
    // —que `entero` rechaza, y su propia prueba lo enumera— seguía valiendo cero.
    //
    // Medido por HTTP: el acta se levantaba igual, sin aviso, declarando el
    // 100 % de la pérdida como DEDUCIBLE cuando quien opera había declarado
    // cobertura de seguro. Eso es una deducción sobredeclarada ante la DIAN.
    //
    // El campo vacío SÍ es cero —no declarar seguro es lo normal—; lo que no
    // puede pasar por cero es lo que no se entiende.
    let tecleado = uno(&cs, "cubierto_por_seguro").trim().to_string();
    let seguro = if tecleado.is_empty() {
        inventario_dominio::dinero::Centavos::CERO
    } else {
        // `v < 0` entra aquí y no aguas abajo: `validar_acta` sí lo rechaza, pero
        // con un mensaje que habla de «supera el costo» —cuando el problema es el
        // signo— y que suelta CENTAVOS CRUDOS en pantalla, o sea una tercera
        // escala en la misma pantalla que acabamos de unificar.
        let Some(v) = entero(&tecleado)
            .filter(|v| *v >= 0)
            .and_then(inventario_dominio::dinero::Centavos::intentar_de_pesos)
        else {
            return pantalla_informes(
                &st,
                &volver(format!(
                    "«{tecleado}» no es un importe válido. Escriba solo dígitos, sin puntos, \
                     comas ni signo."
                )),
            )
            .await;
        };
        v
    };

    let p = inventario_datos::tributario::PeticionActa {
        fecha: uno(&cs, "fecha").to_string(),
        metodo: uno(&cs, "metodo").to_string(),
        representante_legal: uno(&cs, "representante_legal").to_string(),
        // Los responsables van separados por «;», como los guarda el esquema.
        responsables: uno(&cs, "responsables")
            .split(';')
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty())
            .collect(),
        // Mismo caso que `precio_venta`: el rótulo dice «(pesos)» y la columna
        // es centavos, así que quien teclea 5.000 guardaba $50 — y el `CHECK
        // (deducible = costo_total - cubierto_por_seguro)` del esquema restaba
        // dos escalas distintas sin poder detectarlo. El `unwrap_or(0)` se va
        // con él: «ilegible» no es «cero» (directiva 20).
        cubierto_por_seguro: seguro.0,
        renglones,
    };

    match inventario_datos::tributario::crear_acta(&st.pool, &p, Some(s.usuario_id)).await {
        Ok(a) => {
            let m = format!(
                // LOS TRES en la misma escala. Convertí uno solo y la frase
                // dejó de sostenerse a sí misma: «Costo total 3.000.000,
                // cubierto por seguro 1.000, DEDUCIBLE 2.900.000» — y
                // 3.000.000 − 1.000 no es 2.900.000. Lo persistido estaba bien;
                // lo que leía quien opera, no. Y el DEDUCIBLE es la cifra que se
                // lleva a la declaración de renta.
                "Acta {} levantada. Costo total {}, cubierto por seguro {}, DEDUCIBLE {}.",
                a.numero,
                pesos(a.costo_total / 100),
                pesos(a.cubierto_por_seguro / 100),
                pesos(a.deducible / 100)
            );
            pantalla_informes(&st, &volver(m)).await
        }
        Err(e) => pantalla_informes(&st, &volver(format!("No se levantó el acta: {}", mostrable(&e)))).await,
    }
}

fn respuesta_a_entrar() -> Response {
    let mut cab = HeaderMap::new();
    cab.insert(header::LOCATION, "/entrar".parse().expect("ruta válida"));
    (StatusCode::SEE_OTHER, cab).into_response()
}

/// Tener sesión y NO tener permiso es distinto de no haber entrado: mandar a la
/// pantalla de acceso a quien ya entró le hace creer que se le cayó la sesión.
fn prohibido() -> Response {
    (StatusCode::FORBIDDEN, "no tiene permiso para esto").into_response()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// **La versión de htmx vendorizada es la que decimos que es.**
    ///
    /// La directiva 26 pide que htmx viaje dentro del binario «con versión
    /// fijada», y hasta hoy el número no estaba escrito en ninguna parte de
    /// este repositorio: había un archivo minificado de 50 KB y la palabra de
    /// nadie. Un vendorizado sin versión no se puede actualizar por la
    /// directiva 35 —agotar la línea actual antes de saltar— ni cruzar contra
    /// un aviso de seguridad, y un cambio de archivo no se ve en ninguna
    /// revisión.
    ///
    /// Se lee del ARTEFACTO EMBEBIDO, no de un `const` que alguien mantenga al
    /// lado: el que se compara tiene que ser el que se sirve (directiva 31).
    /// La misma que chasqui, informes, medico, nexus y chuspa — comprobado con
    /// `md5sum`, no de memoria: `19a573773be4ca22570ca2f8543120c5`.
    #[test]
    fn htmx_va_vendorizado_en_la_version_declarada() {
        assert!(
            HTMX_JS.contains(r#"version:"2.0.4""#),
            "el htmx embebido NO es 2.0.4: alguien cambió el archivo sin cambiar lo que se declara"
        );
        // PAREJA: la aguja tiene que servir para distinguir. Si buscara algo
        // que cualquier versión trae, pasaría con cualquier archivo.
        assert!(
            !HTMX_JS.contains(r#"version:"1."#),
            "el archivo declara también una versión 1.x: la aguja no discrimina"
        );
        // Y no llega por la red: lo que no esté dentro del binario no existe
        // en una envasadora sin internet.
        assert!(!HTMX_JS.is_empty() && HTMX_JS.len() > 40_000, "htmx embebido incompleto");
    }

    /// **Lo que la pantalla ofrece, el dominio lo acepta.**
    ///
    /// El caso rojo real: la lista de `linea_de_pago` ofrecía «tarjeta», y
    /// `recibo::registrar` solo conoce `tarjeta_credito` y `tarjeta_debito`.
    /// Fallaba ruidosamente —que está bien— pero dejaba a quien opera SIN poder
    /// cobrar con tarjeta desde el panel, y ninguna prueba lo veía porque
    /// cada lado era coherente consigo mismo.
    ///
    /// Esto mide el ARTEFACTO: extrae los `value` del HTML que se sirve y le
    /// pregunta al dominio por cada uno.
    #[test]
    fn todo_medio_de_pago_que_el_panel_ofrece_lo_admite_el_dominio() {
        let html = linea_de_pago().into_string();
        let valores: Vec<&str> = html
            .split(r#"<option value=""#)
            .skip(1)
            .filter_map(|resto| resto.split('"').next())
            .collect();
        assert!(
            valores.len() >= 4,
            "no se extrajo ningún medio del HTML: la aguja dejó de servir, no es que estén bien"
        );
        for v in &valores {
            assert!(
                inventario_dominio::factura::medio_de_pago_conocido(v),
                "el panel ofrece {v:?} y `recibo::registrar` lo rechaza"
            );
        }
    }

    /// PAREJA de la de arriba: el dominio no dice que sí a todo. Sin esto, un
    /// `medio_de_pago_conocido` que devolviera siempre `true` pasaría igual.
    #[test]
    fn el_dominio_rechaza_un_medio_que_no_conoce() {
        assert!(!inventario_dominio::factura::medio_de_pago_conocido("tarjeta"));
        assert!(!inventario_dominio::factura::medio_de_pago_conocido(""));
    }

    /// **La consulta NO sale a pantalla, y ahora hay algo que se pone rojo si
    /// deja de ser verdad.**
    ///
    /// La revisión independiente sobre `5cfe226` lo señaló: de `mostrable`
    /// depende que el texto SQL no vuelva por las diez pantallas nuevas de
    /// `datos_o_aviso!`, y no lo cubría ninguna prueba. La que existía mide
    /// `rutas::fallo`, que es el camino de `/api`, no éste.
    #[test]
    fn un_error_de_sql_no_le_ensena_la_consulta_al_panel() {
        let e = inventario_datos::Error::Sql(sqlx::Error::RowNotFound);
        let m = mostrable(&e);
        assert_eq!(m, "error interno de la base de datos");
        assert!(!m.contains("SELECT") && !m.contains("RowNotFound"));
    }

    /// PAREJA: los errores de REGLA sí se cuentan enteros — son el motivo del
    /// rechazo, y ocultarlos deja a quien opera probando a ciegas contra su
    /// propio sistema. Sin esto, un `mostrable` que devolviera siempre la misma
    /// frase pasaría la de arriba.
    #[test]
    fn un_error_de_regla_si_llega_entero() {
        let e = inventario_datos::Error::Regla("el recibo no puede ser de cero".into());
        assert!(mostrable(&e).contains("el recibo no puede ser de cero"));
    }

    #[test]
    fn los_importes_se_agrupan_por_miles_y_conservan_el_signo() {
        assert_eq!(pesos(0), "0");
        assert_eq!(pesos(999), "999");
        assert_eq!(pesos(1_000), "1.000");
        assert_eq!(pesos(1_234_567), "1.234.567");
        assert_eq!(pesos(-45_000), "-45.000");
    }

    /// La envoltura sirve los tres archivos LOCALES y ninguno de fuera. Es la
    /// misma comprobación que hacía la prueba del `index.html` de Angular, y
    /// sigue haciendo falta por la misma razón: esto corre sin internet.
    #[test]
    fn la_envoltura_no_apunta_a_ningun_origen_externo() {
        let html = envoltura("t", "Empresa", Some("/articulos"), html! { p { "x" } }).into_string();
        assert!(html.contains(r#"src="/htmx.min.js""#));
        assert!(html.contains(r#"src="/boi.js""#));
        assert!(html.contains(r#"href="/panel.css""#));
        assert!(!html.contains("http://"), "la envoltura apunta fuera: {html:.400}");
        assert!(!html.contains("https://"), "la envoltura apunta fuera: {html:.400}");
    }

    /// Sin sesión no se dibuja la barra: un menú visible con las secciones dice
    /// qué hay dentro aunque no se pueda entrar.
    #[test]
    fn la_pantalla_de_entrada_no_lleva_barra_de_navegacion() {
        let html = pagina_de_entrada("Empresa", None, None).into_string();
        assert!(!html.contains("class=\"barra\""), "la entrada trae la barra");
        for (_, nombre) in SECCIONES {
            assert!(!html.contains(nombre), "la entrada nombra la sección «{nombre}»");
        }
    }

    /// La pareja del anterior (directiva 33): con sesión, la barra SÍ está y
    /// marca dónde estamos. Sin esta mitad, una envoltura que no dibujara nunca
    /// nada pasaría igual.
    #[test]
    fn con_sesion_la_barra_esta_y_marca_la_seccion_actual() {
        let html = envoltura("t", "Empresa", Some("/facturar"), html! {}).into_string();
        assert!(html.contains("class=\"barra\""));
        for (_, nombre) in SECCIONES {
            assert!(html.contains(nombre), "falta la sección «{nombre}»");
        }
        assert!(
            html.contains(r#"<a href="/facturar" class="activo">"#),
            "no se marcó la sección actual: {html:.600}"
        );
    }

    #[test]
    fn el_cartel_de_demostracion_solo_sale_con_credenciales() {
        let sin = pagina_de_entrada("E", None, None).into_string();
        assert!(!sin.contains("Demostración pública"));
        let con = pagina_de_entrada("E", Some(("demo".into(), "clave".into())), None).into_string();
        assert!(con.contains("Demostración pública"));
        assert!(con.contains("demo"));
    }

    /// La pareja del arreglo de `entero` (directiva 33): lo que TIENE que leer
    /// y lo que TIENE que rechazar. La versión anterior filtraba a dígitos antes
    /// de parsear, y esos cinco casos de abajo devolvían un número inventado.
    #[test]
    fn entero_rechaza_la_basura_en_vez_de_cosechar_digitos() {
        // Lo que sí es un entero.
        assert_eq!(entero("0"), Some(0));
        assert_eq!(entero("42"), Some(42));
        assert_eq!(entero("-7"), Some(-7));
        assert_eq!(entero("  15  "), Some(15));

        // Lo que NO lo es, y antes daba un número:
        //   "12abc34" -> 1234        (dígitos cosechados)
        //   "1.234,50" -> 123450     (un importe multiplicado por cien)
        //   "0x10" -> 10, "1e3" -> 13, "1_000" -> 1000
        for basura in ["12abc34", "1.234,50", "1.5", "0x10", "1e3", "1_000", "--5", "1-2", "-", "", "  ", "abc"] {
            assert_eq!(entero(basura), None, "«{basura}» se leyó como número");
        }

        // El desbordamiento lo decide `parse`, no un recorte silencioso.
        assert_eq!(entero("9223372036854775808"), None);
        assert_eq!(entero("9223372036854775807"), Some(i64::MAX));
    }

    /// El formulario de facturar NO puede llevar un campo de tarifa: si lo
    /// lleva, la manda el cliente y no el registro del artículo. Esta prueba
    /// mira el HTML tejido, que es donde estaba el defecto (`value="19"` fijo).
    #[test]
    fn el_renglon_de_factura_no_deja_teclear_la_tarifa_de_iva() {
        let art = articulo::Articulo {
            id: 1,
            codigo: "A1".into(),
            codigo_barras: None,
            descripcion: "Botellón".into(),
            presentacion: "und".into(),
            categoria_id: None,
            iva_porcentaje: 5,
            precio_venta: 1000,
            punto_minimo: 0,
            punto_maximo: 0,
            inc_porcentaje: 0,
            unidad_medida: None,
            codigo_estandar: None,
            tipo_estandar: None,
            precio_venta_estimado: None,
            costos_para_vender: None,
            activo: true,
            existencia: 0,
            costo_promedio: None,
            falta_para_factura_electronica: Vec::new(),
        };
        let html = linea_de_factura(std::slice::from_ref(&art)).into_string();

        assert!(
            !html.contains("linea_iva"),
            "el renglón trae un campo de tarifa: la manda el cliente, no el artículo"
        );
        assert!(!html.contains(r#"value="19""#), "quedó el 19 fijo para todo artículo");
        // Y la pareja: el renglón SÍ sigue dejando teclear lo que es de la
        // VENTA —cantidad, precio y descuento—. Sin esta mitad, un renglón vacío
        // pasaría igual.
        for campo in ["linea_articulo", "linea_cantidad", "linea_precio", "linea_descuento"] {
            assert!(html.contains(campo), "falta el campo «{campo}»");
        }
    }
}
