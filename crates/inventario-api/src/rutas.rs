//! Las rutas HTTP.
//!
//! Un principio que no es cosmético: **el código de estado tiene que
//! distinguir**. Un 500 para todo convierte «no hay existencia» —que el cajero
//! puede resolver— en «se rompió el sistema» —que no—, y a la tercera vez nadie
//! lee el mensaje. `estado_de` traduce cada error del dominio al código que le
//! corresponde, y `cada_error_tiene_su_codigo` lo comprueba.

use crate::{claves, sesion};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use inventario_datos::{
    Error, Pool, alistamiento, articulo, catalogo, deterioro, factura, gestion, kardex, nota,
    recibo, resolucion, tercero, tributario, usuario,
};
use inventario_dominio::{Cantidad, Centavos};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct Estado {
    pub pool: Pool,
    pub clave_firma: Arc<Vec<u8>>,
    pub cookie_segura: bool,
    pub horas_sesion: i64,
}

/// Traduce un error a su código HTTP.
///
/// La regla, en una frase: **lo que el usuario puede arreglar es 4xx; lo que
/// solo puede arreglar el programa es 5xx.** No hay «en cualquier otro caso»
/// que se lo trague todo: `Sql` es el único 500, y es 500 porque una consulta
/// que falla es un defecto nuestro.
pub fn estado_de(e: &Error) -> StatusCode {
    match e {
        Error::NoExiste { .. } => StatusCode::NOT_FOUND,
        // 409: el estado del sistema impide la operación, y puede cambiar.
        // No es 400 (la petición está bien escrita) ni 500 (no hay avería).
        Error::Kardex(_) | Error::Numeracion(_) | Error::Cartera(_) => StatusCode::CONFLICT,
        // 422: la petición se entiende pero sus datos no son válidos.
        Error::Factura(_) | Error::Regla(_) => StatusCode::UNPROCESSABLE_ENTITY,
        Error::Sql(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn fallo(e: Error) -> Response {
    let codigo = estado_de(&e);
    // Un fallo de SQL no se le enseña al navegador: llevaría dentro el texto de
    // la consulta. Se registra y se responde algo genérico.
    let mensaje = match &e {
        Error::Sql(interno) => {
            eprintln!("[error] sql: {interno}");
            "error interno de la base de datos".to_string()
        }
        otro => otro.to_string(),
    };
    (codigo, Json(serde_json::json!({ "error": mensaje }))).into_response()
}

fn ok<T: serde::Serialize>(v: T) -> Response {
    (StatusCode::OK, Json(v)).into_response()
}

fn resultado<T: serde::Serialize>(r: Result<T, Error>) -> Response {
    match r {
        Ok(v) => ok(v),
        Err(e) => fallo(e),
    }
}

// ---------------------------------------------------------------------------
// Sesión
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Entrada {
    codigo: String,
    clave: String,
}

async fn entrar(State(st): State<Estado>, Json(e): Json<Entrada>) -> Response {
    let encontrado = match usuario::por_codigo(&st.pool, &e.codigo).await {
        Ok(v) => v,
        Err(err) => return fallo(err),
    };

    // El mismo mensaje y el mismo código tanto si el usuario no existe como si
    // la clave está mal: distinguirlos regala la lista de usuarios válidos.
    let malo = || {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "usuario o clave incorrectos" })),
        )
            .into_response()
    };

    let Some((u, hash)) = encontrado else {
        return malo();
    };
    if !claves::verificar(&e.clave, &hash) {
        return malo();
    }

    let vence = ahora() + st.horas_sesion * 3600;
    let cookie = sesion::emitir(
        &st.clave_firma,
        &sesion::Sesion {
            usuario_id: u.id,
            rol: u.rol.clone(),
            vence,
        },
    );
    let mut h = HeaderMap::new();
    h.insert(
        header::SET_COOKIE,
        sesion::cabecera(&cookie, st.horas_sesion * 3600, st.cookie_segura)
            .parse()
            .expect("cabecera válida"),
    );
    (StatusCode::OK, h, Json(u)).into_response()
}

async fn salir(State(st): State<Estado>) -> Response {
    let mut h = HeaderMap::new();
    h.insert(
        header::SET_COOKIE,
        sesion::cabecera_de_salida(st.cookie_segura)
            .parse()
            .expect("cabecera válida"),
    );
    (StatusCode::OK, h, Json(serde_json::json!({"ok": true}))).into_response()
}

pub fn ahora() -> i64 {
    chrono::Utc::now().timestamp()
}

pub(crate) fn hoy() -> String {
    chrono::Utc::now().date_naive().to_string()
}

/// Lee la sesión de las cabeceras. `None` si no hay o no vale.
pub(crate) fn sesion_de(st: &Estado, h: &HeaderMap) -> Option<sesion::Sesion> {
    let cookie = h.get(header::COOKIE)?.to_str().ok()?;
    sesion::leer(&st.clave_firma, sesion::de_cabecera(cookie)?, ahora())
}

/// Exige sesión y permiso. Devuelve el error ya formado si no lo hay.
///
/// El `Err` lleva una `Response` entera —que clippy señala por grande— y es a
/// propósito: la alternativa es un enum de motivos que cada llamante tenga que
/// volver a traducir a HTTP, y ahí es donde uno se equivoca y devuelve 401
/// donde tocaba 403. El coste es una copia en el camino de error, que por
/// definición no es el camino caliente.
#[allow(clippy::result_large_err)]
fn exigir(st: &Estado, h: &HeaderMap, p: usuario::Permiso) -> Result<sesion::Sesion, Response> {
    let Some(s) = sesion_de(st, h) else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "hace falta iniciar sesión" })),
        )
            .into_response());
    };
    // `puede` devuelve `None` para un rol que no conoce, y ahí se NIEGA: es la
    // única lectura segura de «no sé» en un control de acceso.
    match usuario::puede(&s.rol, p) {
        Some(true) => Ok(s),
        Some(false) => Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": format!("el rol «{}» no puede hacer esto", s.rol) })),
        )
            .into_response()),
        None => Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": format!("rol desconocido: «{}»", s.rol) })),
        )
            .into_response()),
    }
}

async fn quien_soy(State(st): State<Estado>, h: HeaderMap) -> Response {
    match sesion_de(&st, &h) {
        Some(s) => ok(serde_json::json!({ "usuario_id": s.usuario_id, "rol": s.rol })),
        None => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "sin sesión" })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Artículos
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Busqueda {
    #[serde(default)]
    q: String,
    #[serde(default)]
    inactivos: bool,
    #[serde(default)]
    solo: Option<String>,
}

async fn articulos(State(st): State<Estado>, h: HeaderMap, Query(q): Query<Busqueda>) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(articulo::listar(&st.pool, &q.q, q.inactivos).await)
}

async fn articulo_uno(State(st): State<Estado>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(articulo::uno(&st.pool, id).await)
}

async fn articulo_crear(
    State(st): State<Estado>,
    h: HeaderMap,
    Json(n): Json<articulo::Nuevo>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return r;
    }
    match articulo::crear(&st.pool, &n).await {
        Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response(),
        Err(e) => fallo(e),
    }
}

async fn articulo_editar(
    State(st): State<Estado>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(n): Json<articulo::Nuevo>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return r;
    }
    resultado(articulo::actualizar(&st.pool, id, &n).await.map(|_| serde_json::json!({"ok": true})))
}

async fn bajo_minimo(State(st): State<Estado>, h: HeaderMap) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(kardex::bajo_minimo(&st.pool).await)
}

// ---------------------------------------------------------------------------
// Terceros
// ---------------------------------------------------------------------------

async fn terceros(State(st): State<Estado>, h: HeaderMap, Query(q): Query<Busqueda>) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(tercero::listar(&st.pool, &q.q, q.solo.as_deref()).await)
}

async fn tercero_crear(
    State(st): State<Estado>,
    h: HeaderMap,
    Json(n): Json<tercero::Nuevo>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return r;
    }
    match tercero::crear(&st.pool, &n).await {
        Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response(),
        Err(e) => fallo(e),
    }
}

async fn tercero_cartera(State(st): State<Estado>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(factura::por_cobrar(&st.pool, id).await)
}

// ---------------------------------------------------------------------------
// Kardex
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct MovimientoEntrante {
    fecha: String,
    concepto_id: i64,
    articulo_id: i64,
    #[serde(default)]
    tercero_id: Option<i64>,
    #[serde(default)]
    documento: String,
    cantidad: i64,
    #[serde(default)]
    costo_unitario: i64,
    /// Valor comercial **unitario**, obligatorio en los retiros gravados
    /// (ET art. 421 lit. b). Sin él el movimiento se rechaza.
    #[serde(default)]
    valor_comercial_unitario: Option<i64>,
}

async fn movimiento_crear(
    State(st): State<Estado>,
    h: HeaderMap,
    Json(m): Json<MovimientoEntrante>,
) -> Response {
    let s = match exigir(&st, &h, usuario::Permiso::MoverInventario) {
        Ok(s) => s,
        Err(r) => return r,
    };
    let p = kardex::Peticion {
        fecha: m.fecha,
        concepto_id: m.concepto_id,
        articulo_id: m.articulo_id,
        tercero_id: m.tercero_id,
        documento: m.documento,
        cantidad: Cantidad(m.cantidad),
        costo_unitario: Centavos(m.costo_unitario),
        factura_id: None,
        usuario_id: Some(s.usuario_id),
        valor_comercial_unitario: m.valor_comercial_unitario.map(Centavos),
    };
    match kardex::registrar(&st.pool, &p).await {
        Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response(),
        Err(e) => fallo(e),
    }
}

async fn kardex_de(State(st): State<Estado>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(kardex::historia(&st.pool, id, 1000).await)
}

async fn saldo_de(State(st): State<Estado>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(kardex::saldo(&st.pool, id).await)
}

// ---------------------------------------------------------------------------
// Facturación y cartera
// ---------------------------------------------------------------------------

async fn facturar(
    State(st): State<Estado>,
    h: HeaderMap,
    Json(p): Json<factura::Peticion>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Facturar) {
        return r;
    }
    match factura::emitir(&st.pool, &p).await {
        Ok(f) => (StatusCode::CREATED, Json(f)).into_response(),
        Err(e) => fallo(e),
    }
}

#[derive(Deserialize)]
struct Anulacion {
    motivo: String,
}

async fn factura_anular(
    State(st): State<Estado>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(a): Json<Anulacion>,
) -> Response {
    let s = match exigir(&st, &h, usuario::Permiso::Anular) {
        Ok(s) => s,
        Err(r) => return r,
    };
    resultado(
        factura::anular(&st.pool, id, &a.motivo, Some(s.usuario_id))
            .await
            .map(|_| serde_json::json!({"ok": true})),
    )
}

async fn cartera(State(st): State<Estado>, h: HeaderMap) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(factura::cartera(&st.pool, &hoy()).await)
}

async fn recaudar(
    State(st): State<Estado>,
    h: HeaderMap,
    Json(p): Json<recibo::Peticion>,
) -> Response {
    let s = match exigir(&st, &h, usuario::Permiso::Recaudar) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match recibo::registrar(&st.pool, &p, Some(s.usuario_id)).await {
        Ok(r) => (StatusCode::CREATED, Json(r)).into_response(),
        Err(e) => fallo(e),
    }
}

// ---------------------------------------------------------------------------
// Catálogos y configuración
// ---------------------------------------------------------------------------

async fn catalogo_listar(State(st): State<Estado>, h: HeaderMap, Path(cual): Path<String>) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    match catalogo::Tabla::desde_ruta(&cual) {
        Some(t) => resultado(catalogo::listar(&st.pool, t).await),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("no hay catálogo «{cual}»") })),
        )
            .into_response(),
    }
}

async fn conceptos(State(st): State<Estado>, h: HeaderMap) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(catalogo::conceptos(&st.pool).await)
}

async fn resoluciones(State(st): State<Estado>, h: HeaderMap) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(resolucion::listar(&st.pool, &hoy()).await)
}

async fn avisos(State(st): State<Estado>, h: HeaderMap) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(resolucion::por_agotarse(&st.pool, &hoy(), 50).await)
}

async fn resolucion_crear(
    State(st): State<Estado>,
    h: HeaderMap,
    Json(n): Json<resolucion::Nueva>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return r;
    }
    match resolucion::crear(&st.pool, &n).await {
        Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response(),
        Err(e) => fallo(e),
    }
}

#[derive(Deserialize)]
pub(crate) struct EmpresaEntrante {
    pub(crate) nombre: String,
    pub(crate) nit: String,
    #[serde(default)]
    pub(crate) direccion: String,
    #[serde(default)]
    pub(crate) telefono: String,
    #[serde(default)]
    pub(crate) permite_negativo: bool,
    #[serde(default = "promedio")]
    pub(crate) formula_costeo: String,
    #[serde(default = "pruebas")]
    pub(crate) ambiente_dian: String,
    #[serde(default)]
    pub(crate) pin_software: String,
    #[serde(default)]
    pub(crate) id_software: String,
}
fn promedio() -> String {
    "promedio_ponderado".into()
}
fn pruebas() -> String {
    "pruebas".into()
}

/// La empresa completa, **con sesión y permiso de configuración**.
async fn empresa_configuracion(State(st): State<Estado>, h: HeaderMap) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return r;
    }
    match catalogo::empresa(&st.pool).await {
        Ok(Some(e)) => ok(e),
        Ok(None) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "la instalación no está configurada" })),
        )
            .into_response(),
        Err(e) => fallo(e),
    }
}

async fn empresa_guardar(
    State(st): State<Estado>,
    h: HeaderMap,
    Json(e): Json<EmpresaEntrante>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return r;
    }
    match guardar_empresa(&st, &e).await {
        Ok(nit) => ok(serde_json::json!({ "ok": true, "nit": nit })),
        Err(err) => fallo(err),
    }
}

/// Valida y persiste la empresa. Vive APARTE del manejador porque la usan dos
/// entradas —`PUT /api/empresa` y el formulario del panel—, y duplicar aquí la
/// normalización del NIT o la validación de la fórmula de costeo sería crear un
/// segundo camino que se olvida de actualizar (directiva 1).
///
/// Devuelve el NIT ya normalizado con su dígito de verificación.
pub(crate) async fn guardar_empresa(
    st: &Estado,
    e: &EmpresaEntrante,
) -> Result<String, inventario_datos::Error> {
    if e.nombre.trim().is_empty() {
        return Err(inventario_datos::Error::Regla("la empresa necesita nombre".into()));
    }
    let Some(limpio) = inventario_dominio::nit::normalizar(&e.nit) else {
        return Err(inventario_datos::Error::Regla(format!("«{}» no es un NIT", e.nit)));
    };
    let dv = inventario_dominio::nit::digito_verificacion(&limpio).unwrap_or(0);
    // La fórmula de costeo se valida en el dominio para que UEPS reciba SU
    // error —el que cita la norma— y no un «CHECK constraint failed».
    //
    // Y se guarda la forma CANÓNICA que devuelve el dominio, no la que llegó.
    // `desde_texto` es permisivo a propósito —acepta «promedio», «ppp»,
    // «fifo»—, pero el `CHECK` del esquema solo admite «promedio_ponderado» y
    // «peps». Ligar la cadena cruda hacía que el panel, cuyo desplegable manda
    // `promedio`, guardara con un 500 opaco: la validación pasaba y SQLite
    // rechazaba. Son dos vocabularios para lo mismo, y quien los traduce es el
    // dominio (directiva 20: nadie adivina en el camino).
    let formula = match inventario_dominio::costeo::Formula::desde_texto(&e.formula_costeo) {
        Ok(f) => f.como_texto(),
        Err(err) => return Err(Error::Regla(err.to_string())),
    };
    if e.ambiente_dian != "produccion" && e.ambiente_dian != "pruebas" {
        return Err(Error::Regla(format!(
            "ambiente DIAN {:?}: «produccion» o «pruebas»",
            e.ambiente_dian
        )));
    }
    let r = sqlx::query(
        "INSERT INTO empresa (id, nombre, nit, dv, direccion, telefono, permite_negativo,
                              formula_costeo, ambiente_dian, pin_software, id_software)
         VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO UPDATE SET nombre = excluded.nombre, nit = excluded.nit,
             dv = excluded.dv, direccion = excluded.direccion, telefono = excluded.telefono,
             permite_negativo = excluded.permite_negativo,
             formula_costeo = excluded.formula_costeo, ambiente_dian = excluded.ambiente_dian,
             pin_software = excluded.pin_software, id_software = excluded.id_software",
    )
    .bind(e.nombre.trim())
    .bind(&limpio)
    .bind(dv as i64)
    .bind(e.direccion.trim())
    .bind(e.telefono.trim())
    .bind(i64::from(e.permite_negativo))
    .bind(formula)
    .bind(e.ambiente_dian.trim())
    .bind(e.pin_software.trim())
    .bind(e.id_software.trim())
    .execute(&st.pool)
    .await;
    match r {
        Ok(_) => Ok(format!("{limpio}-{dv}")),
        Err(e) => Err(inventario_datos::Error::Sql(e)),
    }
}

/// La empresa **sin sesión**: solo nombre y NIT.
///
/// La pantalla de acceso la necesita antes de que nadie haya entrado. Devuelve
/// `EmpresaPublica` y no `Empresa` a propósito: lo que salga por aquí es
/// público, y el PIN del software es una credencial de la DIAN.
async fn empresa(State(st): State<Estado>) -> Response {
    match catalogo::empresa_publica(&st.pool).await {
        Ok(Some(e)) => ok(e),
        Ok(None) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": "la instalación no está configurada" })),
        )
            .into_response(),
        Err(e) => fallo(e),
    }
}

// ---------------------------------------------------------------------------
// Cumplimiento contable y tributario
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Periodo {
    desde: String,
    hasta: String,
}

#[derive(Deserialize)]
struct FechaCorte {
    fecha: String,
}

/// Corte de deterioro (NIC 2.28-33). Es una ESCRITURA: reconoce la rebaja o su
/// reversión, así que exige permiso de configuración, no de consulta.
async fn deterioro_correr(
    State(st): State<Estado>,
    h: HeaderMap,
    Json(f): Json<FechaCorte>,
) -> Response {
    let s = match exigir(&st, &h, usuario::Permiso::Configurar) {
        Ok(s) => s,
        Err(r) => return r,
    };
    resultado(deterioro::correr_corte(&st.pool, &f.fecha, Some(s.usuario_id)).await)
}

async fn deterioro_leer(State(st): State<Estado>, h: HeaderMap, Path(fecha): Path<String>) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(deterioro::leer_corte(&st.pool, &fecha).await)
}

async fn informe_art64(State(st): State<Estado>, h: HeaderMap, Query(q): Query<Periodo>) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(tributario::art64(&st.pool, &q.desde, &q.hasta).await)
}

async fn informe_retiros(State(st): State<Estado>, h: HeaderMap, Query(q): Query<Periodo>) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(tributario::retiros_gravados(&st.pool, &q.desde, &q.hasta).await)
}

async fn acta_crear(
    State(st): State<Estado>,
    h: HeaderMap,
    Json(p): Json<tributario::PeticionActa>,
) -> Response {
    let s = match exigir(&st, &h, usuario::Permiso::MoverInventario) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match tributario::crear_acta(&st.pool, &p, Some(s.usuario_id)).await {
        Ok(a) => (StatusCode::CREATED, Json(a)).into_response(),
        Err(e) => fallo(e),
    }
}

// ---------------------------------------------------------------------------
// Gestión
// ---------------------------------------------------------------------------

async fn informe_rotacion(State(st): State<Estado>, h: HeaderMap, Query(q): Query<Periodo>) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(gestion::rotacion(&st.pool, &q.desde, &q.hasta).await)
}

async fn informe_abc(State(st): State<Estado>, h: HeaderMap, Query(q): Query<Periodo>) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(gestion::abc(&st.pool, &q.desde, &q.hasta).await)
}

#[derive(Deserialize)]
struct ParametrosReorden {
    desde: String,
    hasta: String,
    #[serde(default = "cinco")]
    dias_reposicion: i64,
    #[serde(default = "noventa_y_cinco")]
    nivel_servicio: i64,
}
fn cinco() -> i64 {
    5
}
fn noventa_y_cinco() -> i64 {
    95
}

async fn informe_reorden(
    State(st): State<Estado>,
    h: HeaderMap,
    Query(q): Query<ParametrosReorden>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(
        gestion::punto_reorden(
            &st.pool,
            &q.desde,
            &q.hasta,
            q.dias_reposicion,
            q.nivel_servicio,
        )
        .await,
    )
}

/// **El diagnóstico de alistamiento**: qué falta para poder operar.
///
/// Se sirve con permiso de CONSULTA y no de configuración: quien va a operar
/// tiene derecho a saber por qué el sistema no le deja hacer algo, sin tener que
/// pedirle al administrador que lo mire.
async fn alistamiento_ver(State(st): State<Estado>, h: HeaderMap) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(alistamiento::diagnosticar(&st.pool, &hoy()).await)
}

// ---------------------------------------------------------------------------
// Notas crédito y débito
// ---------------------------------------------------------------------------

async fn nota_crear(State(st): State<Estado>, h: HeaderMap, Json(p): Json<nota::Peticion>) -> Response {
    // Una nota corrige una factura: es tan sensible como anularla.
    let s = match exigir(&st, &h, usuario::Permiso::Anular) {
        Ok(s) => s,
        Err(r) => return r,
    };
    match nota::emitir(&st.pool, &p, Some(s.usuario_id)).await {
        Ok(n) => (StatusCode::CREATED, Json(n)).into_response(),
        Err(e) => fallo(e),
    }
}

async fn notas_de_factura(State(st): State<Estado>, h: HeaderMap, Path(id): Path<i64>) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Consultar) {
        return r;
    }
    resultado(nota::de_factura(&st.pool, id).await)
}

// ---------------------------------------------------------------------------
// Usuarios y catálogos de escritura
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct UsuarioNuevo {
    codigo: String,
    nombre: String,
    clave: String,
    rol: String,
}

async fn usuarios_listar(State(st): State<Estado>, h: HeaderMap) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return r;
    }
    resultado(usuario::listar(&st.pool).await)
}

async fn usuario_crear(
    State(st): State<Estado>,
    h: HeaderMap,
    Json(u): Json<UsuarioNuevo>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return r;
    }
    // La clave se exige larga aquí y no en `datos`: es una política de la
    // instalación, no una regla del modelo.
    if u.clave.len() < 8 {
        return fallo(Error::Regla("la clave debe tener al menos 8 caracteres".into()));
    }
    let Some(hash) = claves::hashear(&u.clave) else {
        return fallo(Error::Regla("no se pudo generar el hash de la clave".into()));
    };
    match usuario::crear(&st.pool, &u.codigo, &u.nombre, &hash, &u.rol).await {
        Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response(),
        Err(e) => fallo(e),
    }
}

#[derive(Deserialize)]
struct ConceptoNuevo {
    codigo: String,
    nombre: String,
    signo: String,
    clase: String,
    efecto_tributario: String,
    #[serde(default)]
    exige_tercero: bool,
}

async fn concepto_crear(
    State(st): State<Estado>,
    h: HeaderMap,
    Json(c): Json<ConceptoNuevo>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return r;
    }
    match catalogo::crear_concepto(
        &st.pool,
        &c.codigo,
        &c.nombre,
        &c.signo,
        &c.clase,
        &c.efecto_tributario,
        c.exige_tercero,
    )
    .await
    {
        Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response(),
        Err(e) => fallo(e),
    }
}

#[derive(Deserialize)]
struct EfectoEntrante {
    efecto_tributario: String,
}

/// Corrige el efecto tributario de un concepto ya existente.
///
/// Es la contrapartida del aviso que el migrador deja en su informe: sin este
/// camino, «se reporta para revisión humana» era una promesa que el sistema no
/// podía cumplir.
async fn concepto_efecto(
    State(st): State<Estado>,
    h: HeaderMap,
    Path(id): Path<i64>,
    Json(e): Json<EfectoEntrante>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return r;
    }
    match catalogo::actualizar_efecto_tributario(&st.pool, id, &e.efecto_tributario).await {
        // `movimientos_ya_registrados` viaja en la respuesta a propósito: la
        // corrección vale hacia adelante, y quien la hace tiene que ver si
        // llegó después de que el concepto ya moviera kardex.
        Ok(ya) => Json(serde_json::json!({
            "ok": true,
            "movimientos_ya_registrados": ya
        }))
        .into_response(),
        Err(e) => fallo(e),
    }
}

#[derive(Deserialize)]
struct ItemCatalogo {
    codigo: String,
    nombre: String,
}

async fn catalogo_crear(
    State(st): State<Estado>,
    h: HeaderMap,
    Path(cual): Path<String>,
    Json(i): Json<ItemCatalogo>,
) -> Response {
    if let Err(r) = exigir(&st, &h, usuario::Permiso::Configurar) {
        return r;
    }
    match catalogo::Tabla::desde_ruta(&cual) {
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("no hay catálogo «{cual}»") })),
        )
            .into_response(),
        Some(t) => match catalogo::crear(&st.pool, t, &i.codigo, &i.nombre).await {
            Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response(),
            Err(e) => fallo(e),
        },
    }
}

/// **Las credenciales de la DEMOSTRACIÓN**, para que la pantalla de acceso las
/// muestre.
///
/// Responde `204 Sin contenido` cuando esta instalación no es una demostración,
/// que es el caso de cualquier cliente. Es la única diferencia entre el binario
/// que va al servidor público y el que va a un mostrador: dos variables de
/// entorno, no dos compilaciones.
async fn demo_credenciales() -> Response {
    match crate::demo::credenciales() {
        Some((usuario, clave)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "usuario": usuario,
                "clave": clave,
                // `concat!` y no una cadena partida en dos líneas. MEDIDO el
                // 2026-08-11, porque la explicación que había aquí era la
                // equivocada y llevaba a evitar el construido seguro:
                //   sin `\` al final  → conserva el salto Y la sangría del
                //                       código («uno dos\n         tres»), que
                //                       es el que sacó este aviso a producción
                //                       con veinte espacios en medio;
                //   con `\` al final  → junta SIN separador («dostres»), así
                //                       que el espacio hay que dejarlo antes
                //                       de la barra o se pegan dos palabras.
                // Los dos muerden en silencio. `concat!` no tiene ninguno de
                // los dos filos: cada trozo es una cadena entera y se ve.
                "aviso": concat!(
                    "Demostración pública: cualquiera puede escribir en esta base y se ",
                    "reinicia periódicamente. No emite documentos con validez fiscal."
                )
            })),
        )
            .into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

/// Comprobación de vida. Sin sesión: la usa quien vigila el proceso.
async fn salud(State(st): State<Estado>) -> Response {
    match sqlx::query("SELECT 1").execute(&st.pool).await {
        Ok(_) => ok(serde_json::json!({ "estado": "vivo" })),
        Err(e) => {
            eprintln!("[error] salud: {e}");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "estado": "sin base" })),
            )
                .into_response()
        }
    }
}

pub fn router(estado: Estado) -> Router {
    Router::new()
        .route("/api/salud", get(salud))
        .route("/api/demo", get(demo_credenciales))
        .route("/api/empresa", get(empresa).put(empresa_guardar))
        .route("/api/empresa/configuracion", get(empresa_configuracion))
        .route("/api/sesion", post(entrar).delete(salir))
        .route("/api/sesion/quien", get(quien_soy))
        .route("/api/articulos", get(articulos).post(articulo_crear))
        .route("/api/articulos/{id}", get(articulo_uno).put(articulo_editar))
        .route("/api/articulos/{id}/kardex", get(kardex_de))
        .route("/api/articulos/{id}/saldo", get(saldo_de))
        .route("/api/informes/bajo-minimo", get(bajo_minimo))
        .route("/api/informes/cartera", get(cartera))
        .route("/api/informes/avisos", get(avisos))
        .route("/api/informes/art64", get(informe_art64))
        .route("/api/informes/retiros", get(informe_retiros))
        .route("/api/informes/rotacion", get(informe_rotacion))
        .route("/api/informes/abc", get(informe_abc))
        .route("/api/informes/reorden", get(informe_reorden))
        .route("/api/deterioro", post(deterioro_correr))
        .route("/api/deterioro/{fecha}", get(deterioro_leer))
        .route("/api/actas", post(acta_crear))
        .route("/api/terceros", get(terceros).post(tercero_crear))
        .route("/api/terceros/{id}/cartera", get(tercero_cartera))
        .route("/api/movimientos", post(movimiento_crear))
        .route("/api/facturas", post(facturar))
        .route("/api/facturas/{id}/anular", post(factura_anular))
        .route("/api/recibos", post(recaudar))
        .route("/api/alistamiento", get(alistamiento_ver))
        .route("/api/notas", post(nota_crear))
        .route("/api/facturas/{id}/notas", get(notas_de_factura))
        .route("/api/usuarios", get(usuarios_listar).post(usuario_crear))
        .route("/api/conceptos", get(conceptos).post(concepto_crear))
        .route("/api/conceptos/{id}/efecto", put(concepto_efecto))
        .route("/api/resoluciones", get(resoluciones).post(resolucion_crear))
        .route("/api/catalogos/{cual}", get(catalogo_listar).post(catalogo_crear))
        // ── El panel, tejido en el servidor con maud (directiva 26) ──
        // Va DESPUÉS de `/api`, y el orden importa: una ruta comodín arriba
        // taparía la API. Lo estático se sirve desde el binario, nunca de un
        // CDN — esto corre en un negocio que puede quedarse sin internet.
        .route("/htmx.min.js", get(crate::vista::htmx_js))
        .route("/boi.js", get(crate::vista::boi_js))
        .route(
            "/comportamientos/sugerir-de-opcion.js",
            get(crate::vista::sugerir_js),
        )
        .route("/boi.css", get(crate::vista::boi_css))
        .route("/panel.css", get(crate::vista::panel_css))
        .route("/entrar", get(crate::vista::entrar_pagina).post(crate::vista::entrar_enviar))
        .route("/salir", post(crate::vista::salir_enviar))
        .route("/articulos", get(crate::vista::articulos_pagina).post(crate::vista::articulos_crear))
        .route("/articulos/tabla", get(crate::vista::articulos_tabla))
        .route("/movimientos", get(crate::vista::movimientos_pagina).post(crate::vista::movimientos_enviar))
        .route("/facturar", get(crate::vista::facturar_pagina).post(crate::vista::facturar_enviar))
        .route("/facturar/linea", get(crate::vista::facturar_linea))
        .route("/recaudar", get(crate::vista::recaudar_pagina).post(crate::vista::recaudar_enviar))
        .route("/recaudar/pago", get(crate::vista::recaudar_pago))
        .route("/terceros", get(crate::vista::terceros_pagina).post(crate::vista::terceros_crear))
        .route("/cartera", get(crate::vista::cartera_pagina))
        .route("/informes", get(crate::vista::informes_pagina))
        .route("/informes/deterioro", post(crate::vista::informes_deterioro))
        .route("/informes/acta", post(crate::vista::informes_acta))
        .route("/alistamiento", get(crate::vista::alistamiento_pagina))
        .route("/configuracion", get(crate::vista::configuracion_pagina))
        .route("/configuracion/empresa", post(crate::vista::configuracion_empresa))
        .route("/configuracion/usuario", post(crate::vista::configuracion_usuario))
        .route("/configuracion/software", post(crate::vista::configuracion_software))
        .route("/configuracion/resolucion", post(crate::vista::configuracion_resolucion))
        .route("/configuracion/concepto", post(crate::vista::configuracion_concepto))
        .route(
            "/configuracion/concepto/efecto",
            post(crate::vista::configuracion_concepto_efecto),
        )
        // La raíz manda a la primera sección; una ruta desconocida, también.
        // Pero un `/api/...` que no existe recibe 404, para que un error no
        // llegue disfrazado de página — que es como un fallo de API acaba
        // apareciendo en pantalla como «no pasa nada».
        .route("/", get(crate::vista::raiz))
        .fallback(crate::vista::no_encontrado)
        // Las cabeceras de seguridad van en la APLICACIÓN, no solo en el nginx
        // de la guía: esto se instala en el servidor del cliente y quien lo
        // administre puede poner otro proxy. Hasta el 2026-08-06 NO había
        // ninguna —ni CSP, ni nosniff, ni frame-options—, y la prueba del
        // `index.html` de Angular decía defender `script-src 'self'`: defendía
        // una política que nunca se aplicó.
        .layer(axum::middleware::from_fn(cabeceras_de_seguridad))
        .with_state(estado)
}

/// `script-src 'self'` es lo que de verdad para un XSS, y es lo que hace que
/// `boi` tenga que viajar vendorizado: con un CDN habría que abrirla.
/// El `Origin` de una petición que escribe, comparado con el `Host` al que
/// llegó. `None` = no vino, que NO es lo mismo que no coincidir.
///
/// Devuelve `Some(false)` solo cuando vino un `Origin` y es de otro sitio.
fn origen_ajeno(cabeceras: &HeaderMap) -> bool {
    let Some(origen) = cabeceras.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        // Sin `Origin` no se bloquea, y es deliberado (directiva 21): lo omiten
        // los clientes que NO son navegadores —un `curl`, la integración de un
        // tercero contra `/api`, el arnés de pruebas—, y ésos son operación
        // normal. Lo que se rechaza es un origen que se declara y no es el
        // nuestro, que ningún cliente legítimo produce.
        return false;
    };
    let Some(host) = cabeceras.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    // `Origin` es «esquema://host[:puerto]» y `Host` es «host[:puerto]»: detrás
    // de un proxy el esquema no llega, así que se compara la autoridad.
    let autoridad = origen.split_once("://").map(|(_, a)| a).unwrap_or(origen);
    autoridad != host
}

async fn cabeceras_de_seguridad(
    peticion: axum::extract::Request,
    siguiente: axum::middleware::Next,
) -> Response {
    // **Freno de origen para lo que ESCRIBE.** `SameSite=Strict` en la cookie
    // sigue siendo el control principal y no cambia; esto cubre lo que aquél no
    // ve, porque `SameSite` es por SITIO y no por ORIGEN: en el VPS conviven
    // `inventario`, `chasqui`, `medico` e `informes` bajo `xiliux.com`, y un
    // formulario servido desde un hermano viaja como «mismo sitio» con la
    // cookie puesta. Sin esto, un XSS en el vecino escribe aquí.
    //
    // Va en el middleware y no en cada manejador a propósito: es un solo sitio,
    // y cubre las rutas de mañana igual que las de hoy.
    if !matches!(peticion.method(), &Method::GET | &Method::HEAD | &Method::OPTIONS)
        && origen_ajeno(peticion.headers())
    {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "origen no permitido" })),
        )
            .into_response();
    }

    let mut r = siguiente.run(peticion).await;
    let h = r.headers_mut();
    h.insert("x-content-type-options", "nosniff".parse().expect("valor válido"));
    h.insert("x-frame-options", "DENY".parse().expect("valor válido"));
    h.insert("referrer-policy", "same-origin".parse().expect("valor válido"));
    // El panel sirve HTML con nombres de terceros, saldos y el censo de usuarios,
    // por GET y sin validadores: un proxy compartido puede guardarlo por
    // heurística. Con JSON el riesgo era menor porque no se cachea igual; con
    // HTML por usuario, cambia de forma.
    h.insert("cache-control", "no-store".parse().expect("valor válido"));
    h.insert(
        "content-security-policy",
        "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
         font-src 'self' data:; object-src 'none'; form-action 'self'; frame-ancestors 'none'; base-uri 'none'"
            .parse()
            .expect("valor válido"),
    );
    r
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Directiva 23: la regla en una frase, sin «en cualquier otro caso». Si
    /// mañana se añade una variante a `Error` y nadie decide su código, esta
    /// prueba no compila — que es exactamente lo que se quiere.
    #[test]
    fn cada_error_tiene_su_codigo_y_no_todos_son_quinientos() {
        use inventario_dominio::{cartera, factura as df, kardex as dk, numeracion as dn};

        let casos: Vec<(Error, StatusCode)> = vec![
            (
                Error::NoExiste { que: "artículo", id: 1 },
                StatusCode::NOT_FOUND,
            ),
            (
                Error::Kardex(dk::ErrorKardex::ExistenciaInsuficiente {
                    hay: Cantidad::CERO,
                    piden: Cantidad(1),
                }),
                StatusCode::CONFLICT,
            ),
            (
                Error::Numeracion(dn::ErrorNumeracion::Agotada { id: 1, hasta: 10 }),
                StatusCode::CONFLICT,
            ),
            (
                Error::Cartera(cartera::ErrorCartera::FacturaAnulada(1)),
                StatusCode::CONFLICT,
            ),
            (
                Error::Factura(df::ErrorFactura::SinLineas),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                Error::Regla("lo que sea".into()),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
            (
                Error::Sql(sqlx::Error::RowNotFound),
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ];
        for (e, esperado) in &casos {
            assert_eq!(estado_de(e), *esperado, "el error «{e}» no da {esperado}");
        }

        // Y que DISCRIMINE: si todo fuera 500, esta comprobación caería.
        let distintos: std::collections::BTreeSet<u16> =
            casos.iter().map(|(e, _)| estado_de(e).as_u16()).collect();
        assert!(
            distintos.len() >= 4,
            "los errores solo producen {} códigos distintos: no distinguen nada",
            distintos.len()
        );

        // Exhaustividad: este `match` no compila si aparece una variante nueva
        // sin decidir su código.
        fn _obliga_a_decidir(e: &Error) {
            match e {
                Error::NoExiste { .. }
                | Error::Kardex(_)
                | Error::Numeracion(_)
                | Error::Cartera(_)
                | Error::Factura(_)
                | Error::Regla(_)
                | Error::Sql(_) => {}
            }
        }
    }

    #[test]
    fn un_fallo_de_sql_no_le_enseña_la_consulta_al_navegador() {
        let r = fallo(Error::Sql(sqlx::Error::Protocol(
            "SELECT clave FROM usuario WHERE ...".into(),
        )));
        assert_eq!(r.status(), StatusCode::INTERNAL_SERVER_ERROR);
        // El cuerpo se comprueba en las pruebas de extremo a extremo; aquí basta
        // con que el camino sea el de `Sql`, que es el único que oculta.
    }
}
