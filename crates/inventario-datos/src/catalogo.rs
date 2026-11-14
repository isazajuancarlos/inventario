//! Catálogos pequeños: conceptos de kardex, categorías, convenios, geografía,
//! vendedores y bancos. Todos con la misma forma: código, nombre, activo.

use crate::{Error, Pool, Resultado};
use sqlx::Row;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Item {
    pub id: i64,
    pub codigo: String,
    pub nombre: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Concepto {
    pub id: i64,
    pub codigo: String,
    pub nombre: String,
    pub signo: String,
    pub clase: String,
    /// **Se lista a propósito.** El migrador deja todo concepto de salida
    /// heredado en `no_afecta_impuestos` y lo marca para revisión humana; si la
    /// pantalla no enseña el valor, esa revisión no se puede hacer y el
    /// obsequio se queda sin IVA para siempre (ET art. 421 lit. b).
    pub efecto_tributario: String,
    pub exige_tercero: bool,
    pub activo: bool,
}

/// Las tablas que este módulo sabe listar. Es una lista **cerrada**: recibir el
/// nombre de la tabla desde fuera y meterlo en el SQL sería inyección, por más
/// que hoy solo la llame nuestra propia API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tabla {
    Categoria,
    Convenio,
    Ciudad,
    Zona,
    Barrio,
    Vendedor,
    Banco,
}

impl Tabla {
    pub fn desde_ruta(s: &str) -> Option<Tabla> {
        Some(match s {
            "categorias" => Tabla::Categoria,
            "convenios" => Tabla::Convenio,
            "ciudades" => Tabla::Ciudad,
            "zonas" => Tabla::Zona,
            "barrios" => Tabla::Barrio,
            "vendedores" => Tabla::Vendedor,
            "bancos" => Tabla::Banco,
            _ => None?,
        })
    }

    fn nombre_sql(self) -> &'static str {
        match self {
            Tabla::Categoria => "categoria",
            Tabla::Convenio => "convenio",
            Tabla::Ciudad => "ciudad",
            Tabla::Zona => "zona",
            Tabla::Barrio => "barrio",
            Tabla::Vendedor => "vendedor",
            Tabla::Banco => "banco",
        }
    }
}

pub async fn listar(pool: &Pool, t: Tabla) -> Resultado<Vec<Item>> {
    let sql = format!("SELECT id, codigo, nombre FROM {} ORDER BY nombre", t.nombre_sql());
    let filas = sqlx::query(&sql).fetch_all(pool).await?;
    Ok(filas
        .into_iter()
        .map(|f| Item {
            id: f.get("id"),
            codigo: f.get("codigo"),
            nombre: f.get("nombre"),
        })
        .collect())
}

pub async fn crear(pool: &Pool, t: Tabla, codigo: &str, nombre: &str) -> Resultado<i64> {
    if codigo.trim().is_empty() || nombre.trim().is_empty() {
        return Err(Error::Regla("código y nombre son obligatorios".into()));
    }
    let sql = format!(
        "INSERT INTO {} (codigo, nombre) VALUES (?, ?) RETURNING id",
        t.nombre_sql()
    );
    Ok(sqlx::query(&sql)
        .bind(codigo.trim())
        .bind(nombre.trim())
        .fetch_one(pool)
        .await?
        .get("id"))
}

pub async fn conceptos(pool: &Pool) -> Resultado<Vec<Concepto>> {
    let filas = sqlx::query(
        "SELECT id, codigo, nombre, signo, clase, efecto_tributario, exige_tercero, activo
           FROM concepto ORDER BY codigo",
    )
    .fetch_all(pool)
    .await?;
    Ok(filas
        .into_iter()
        .map(|f| Concepto {
            id: f.get("id"),
            codigo: f.get("codigo"),
            nombre: f.get("nombre"),
            signo: f.get("signo"),
            clase: f.get("clase"),
            efecto_tributario: f.get("efecto_tributario"),
            exige_tercero: f.get::<i64, _>("exige_tercero") == 1,
            activo: f.get::<i64, _>("activo") == 1,
        })
        .collect())
}

/// Crea un concepto de kardex.
///
/// `signo`, `clase` y `efecto_tributario` son listas cerradas en el esquema.
/// Aquí se validan antes para que el error diga qué se esperaba, en vez de un
/// `CHECK constraint failed` que no ayuda a nadie.
///
/// **`efecto_tributario` no tiene valor por defecto, y eso es deliberado**: al
/// crear un concepto de salida hay que decidir si es una venta, un retiro
/// gravado (ET art. 421 lit. b), un faltante del art. 64, una baja con acta o
/// algo que no toca impuestos. Un defecto aquí dejaría cada obsequio nuevo sin
/// IVA por omisión — que es exactamente lo que le pasó al sistema anterior.
pub async fn crear_concepto(
    pool: &Pool,
    codigo: &str,
    nombre: &str,
    signo: &str,
    clase: &str,
    efecto_tributario: &str,
    exige_tercero: bool,
) -> Resultado<i64> {
    const SIGNOS: [&str; 2] = ["entrada", "salida"];
    const CLASES: [&str; 8] = [
        "compra",
        "venta",
        "devolucion_venta",
        "devolucion_compra",
        "ajuste",
        "traslado",
        "inicial",
        "consumo",
    ];
    if !SIGNOS.contains(&signo) {
        return Err(Error::Regla(format!(
            "signo {signo:?}: solo «entrada» o «salida»"
        )));
    }
    if !CLASES.contains(&clase) {
        return Err(Error::Regla(format!(
            "clase {clase:?}: una de {}",
            CLASES.join(", ")
        )));
    }
    let efecto = inventario_dominio::retiro::EfectoTributario::desde_texto(efecto_tributario)
        .ok_or_else(|| {
            Error::Regla(format!(
                "efecto tributario {efecto_tributario:?} desconocido: uno de venta, \
                 retiro_gravado (ET art. 421 lit. b), faltante_art64, baja_con_acta o \
                 no_afecta_impuestos"
            ))
        })?;
    Ok(sqlx::query(
        "INSERT INTO concepto (codigo, nombre, signo, clase, efecto_tributario, exige_tercero)
         VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(codigo.trim())
    .bind(nombre.trim())
    .bind(signo)
    .bind(clase)
    .bind(efecto.como_texto())
    .bind(i64::from(exige_tercero))
    .fetch_one(pool)
    .await?
    .get("id"))
}

/// Corrige el efecto tributario de un concepto que ya existe.
///
/// **Sin esto, la brecha de mayor riesgo del sistema no se puede cerrar.** El
/// migrador deja todo concepto de salida heredado en `no_afecta_impuestos`
/// —porque el corpus no trae el dato y adivinarlo por el nombre sería una regla
/// frágil— y lo REPORTA para que una persona decida. Hasta hoy no había ningún
/// camino para que esa persona ejecutara la decisión: el obsequio, el consumo
/// interno y el retiro de socios se quedaban sin IVA para siempre, que es
/// exactamente lo que hacía el sistema anterior en 161 movimientos.
///
/// **Corrige hacia adelante, no hacia atrás, y hay que decirlo.** Un movimiento
/// ya registrado guardó su propio impuesto en el momento de escribirse; cambiar
/// el concepto no lo reescribe. Por eso el sitio de esta corrección es la
/// revisión posterior a la migración, ANTES de operar — que es justo cuando el
/// informe del migrador la pide y cuando todavía no hay kardex vivo.
///
/// Devuelve cuántos movimientos YA registrados usan el concepto: cero significa
/// que la corrección llegó a tiempo, y cualquier otra cosa es un dato que quien
/// corrige tiene que ver, no una advertencia que se traga la función.
pub async fn actualizar_efecto_tributario(
    pool: &Pool,
    concepto_id: i64,
    efecto_tributario: &str,
) -> Resultado<i64> {
    let efecto = inventario_dominio::retiro::EfectoTributario::desde_texto(efecto_tributario)
        .ok_or_else(|| {
            Error::Regla(format!(
                "efecto tributario {efecto_tributario:?} desconocido: uno de venta, \
                 retiro_gravado (ET art. 421 lit. b), faltante_art64, baja_con_acta o \
                 no_afecta_impuestos"
            ))
        })?;

    // El movimiento ya escrito no se toca; se CUENTA. Quien corrige tiene que
    // saber si llegó tarde, y un cero silencioso no distingue «no había
    // ninguno» de «no se miró» (directiva 30).
    let ya: i64 = sqlx::query("SELECT COUNT(*) AS n FROM movimiento WHERE concepto_id = ?")
        .bind(concepto_id)
        .fetch_one(pool)
        .await?
        .get("n");

    let r = sqlx::query("UPDATE concepto SET efecto_tributario = ? WHERE id = ?")
        .bind(efecto.como_texto())
        .bind(concepto_id)
        .execute(pool)
        .await?;
    if r.rows_affected() == 0 {
        return Err(Error::Regla(format!(
            "no hay ningún concepto con el id {concepto_id}"
        )));
    }
    Ok(ya)
}

/// Lo que se puede enseñar SIN sesión.
///
/// La pantalla de acceso muestra el nombre del negocio antes de que nadie haya
/// entrado, así que este dato es público por diseño. **Y solo este**: el PIN del
/// software es una credencial que la DIAN asigna al facturador, y servirlo en
/// una ruta abierta sería regalarlo a cualquiera que sepa la dirección.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EmpresaPublica {
    pub nombre: String,
    pub nit: String,
    pub dv: i64,
}

/// Lo que ve quien tiene permiso de configurar. Incluye las credenciales de
/// facturación electrónica.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Empresa {
    pub nombre: String,
    pub nit: String,
    pub dv: i64,
    pub direccion: String,
    pub telefono: String,
    pub permite_negativo: bool,
    pub formula_costeo: String,
    pub ambiente_dian: String,
    pub pin_software: String,
    pub id_software: String,
}

pub async fn empresa_publica(pool: &Pool) -> Resultado<Option<EmpresaPublica>> {
    let f = sqlx::query("SELECT nombre, nit, dv FROM empresa WHERE id = 1")
        .fetch_optional(pool)
        .await?;
    Ok(f.map(|f| EmpresaPublica {
        nombre: f.get("nombre"),
        nit: f.get("nit"),
        dv: f.get("dv"),
    }))
}

pub async fn empresa(pool: &Pool) -> Resultado<Option<Empresa>> {
    let f = sqlx::query(
        "SELECT nombre, nit, dv, direccion, telefono, permite_negativo, formula_costeo,
                ambiente_dian, pin_software, id_software
           FROM empresa WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(f.map(|f| Empresa {
        nombre: f.get("nombre"),
        nit: f.get("nit"),
        dv: f.get("dv"),
        direccion: f.get("direccion"),
        telefono: f.get("telefono"),
        permite_negativo: f.get::<i64, _>("permite_negativo") == 1,
        formula_costeo: f.get("formula_costeo"),
        ambiente_dian: f.get("ambiente_dian"),
        pin_software: f.get::<Option<String>, _>("pin_software").unwrap_or_default(),
        id_software: f.get::<Option<String>, _>("id_software").unwrap_or_default(),
    }))
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::prueba_apoyo;

    #[tokio::test]
    async fn la_lista_de_tablas_es_cerrada() {
        assert_eq!(Tabla::desde_ruta("categorias"), Some(Tabla::Categoria));
        // Lo que no está en la lista no se convierte en SQL.
        assert_eq!(Tabla::desde_ruta("usuario"), None);
        assert_eq!(Tabla::desde_ruta("factura; DROP TABLE factura"), None);
        assert_eq!(Tabla::desde_ruta(""), None);
    }

    #[tokio::test]
    async fn los_catalogos_se_crean_y_se_listan() {
        let pool = prueba_apoyo::base().await;
        crear(&pool, Tabla::Convenio, "01", "Distribuidor").await.unwrap();
        crear(&pool, Tabla::Convenio, "04", "Hogar").await.unwrap();
        let l = listar(&pool, Tabla::Convenio).await.unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].nombre, "Distribuidor", "ordenados por nombre");
        // Código repetido: no.
        assert!(crear(&pool, Tabla::Convenio, "01", "Otro").await.is_err());
        assert!(crear(&pool, Tabla::Convenio, " ", "Sin código").await.is_err());
    }

    #[tokio::test]
    async fn un_concepto_con_signo_o_clase_inventados_se_rechaza_con_un_mensaje_util() {
        let pool = prueba_apoyo::base().await;
        let e = crear_concepto(&pool, "99", "Raro", "arriba", "ajuste", "no_afecta_impuestos", false)
            .await
            .unwrap_err();
        assert!(e.to_string().contains("entrada"), "el error no dice qué se esperaba: {e}");
        let e = crear_concepto(&pool, "99", "Raro", "entrada", "magia", "no_afecta_impuestos", false)
            .await
            .unwrap_err();
        assert!(e.to_string().contains("compra"), "salió {e}");
        // Y el efecto tributario, que es el que decide si se liquida IVA.
        let e = crear_concepto(&pool, "99", "Raro", "salida", "consumo", "ninguno", false)
            .await
            .unwrap_err();
        assert!(e.to_string().contains("421"), "el error no cita la norma: {e}");
        assert!(
            crear_concepto(&pool, "99", "Bueno", "entrada", "ajuste", "no_afecta_impuestos", false)
                .await
                .is_ok()
        );
    }

    /// Un concepto de obsequio tiene que poder declararse como retiro gravado,
    /// y quedar así en la base.
    #[tokio::test]
    async fn un_concepto_de_obsequio_se_declara_como_retiro_gravado() {
        let pool = prueba_apoyo::base().await;
        let id = crear_concepto(&pool, "76", "Salidas por obsequios", "salida", "consumo",
                                "retiro_gravado", false)
            .await
            .unwrap();
        let (e,): (String,) = sqlx::query_as("SELECT efecto_tributario FROM concepto WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(e, "retiro_gravado");
    }

    #[tokio::test]
    async fn los_conceptos_sembrados_cubren_venta_y_devolucion() {
        let pool = prueba_apoyo::base().await;
        let c = conceptos(&pool).await.unwrap();
        assert!(c.iter().any(|x| x.clase == "venta" && x.signo == "salida"));
        assert!(c.iter().any(|x| x.clase == "devolucion_venta" && x.signo == "entrada"));
    }

    #[tokio::test]
    async fn sin_empresa_configurada_se_dice_que_no_hay() {
        let pool = crate::abrir("sqlite::memory:").await.unwrap();
        assert!(empresa(&pool).await.unwrap().is_none());
        assert!(empresa_publica(&pool).await.unwrap().is_none());
        let pool = prueba_apoyo::base().await;
        assert_eq!(empresa(&pool).await.unwrap().unwrap().nit, "811033579");
    }

    /// **La vista pública NO lleva credenciales.** La pantalla de acceso la pide
    /// sin sesión, así que todo lo que salga por ahí es público: el PIN del
    /// software, que la DIAN asigna al facturador, no puede estar.
    #[tokio::test]
    async fn la_vista_publica_de_la_empresa_no_lleva_el_pin_del_software() {
        let pool = prueba_apoyo::base().await;
        sqlx::query("UPDATE empresa SET pin_software = 'SECRETO123', id_software = 'sw-1'")
            .execute(&pool)
            .await
            .unwrap();

        let publica = empresa_publica(&pool).await.unwrap().unwrap();
        let json = serde_json::to_string(&publica).unwrap();
        assert!(!json.contains("SECRETO123"), "el PIN se escapó a la ruta pública: {json}");
        assert!(!json.contains("pin"), "{json}");
        assert!(!json.contains("sw-1"), "{json}");
        assert!(json.contains("811033579"));

        // La vista con sesión sí lo trae: es quien configura.
        assert_eq!(empresa(&pool).await.unwrap().unwrap().pin_software, "SECRETO123");
    }
}
