//! Resoluciones de facturación.
//!
//! El aviso de agotamiento es la razón de ser del módulo: dos de los seis
//! grupos del sistema viejo estaban agotados o al borde (`08`: 5650 de 5650;
//! `05`: 5349 de 5350) y nada lo decía.

use crate::{Error, Pool, Resultado};
use inventario_dominio::numeracion::{self, Resolucion};
use sqlx::Row;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Vista {
    pub id: i64,
    pub nombre: String,
    pub prefijo: String,
    pub numero: String,
    pub fecha: Option<String>,
    pub rige_desde: Option<String>,
    pub vence: Option<String>,
    /// La clave técnica que entrega la DIAN con la autorización. Sin ella las
    /// facturas de esta resolución salen sin CUFE.
    pub clave_tecnica: Option<String>,
    pub desde: i64,
    pub hasta: i64,
    pub ultimo_emitido: Option<i64>,
    pub activa: bool,
    /// Cuántos números quedan. Cero = agotada.
    pub quedan: i64,
    /// Por qué no se puede facturar con ella, si es que no se puede. `None`
    /// cuando sí se puede.
    pub impedimento: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Nueva {
    pub nombre: String,
    #[serde(default)]
    pub prefijo: String,
    #[serde(default)]
    pub numero: String,
    #[serde(default)]
    pub fecha: Option<String>,
    /// Inicio de la vigencia de la autorización. El rango numérico y la
    /// vigencia se agotan **por separado**.
    #[serde(default)]
    pub rige_desde: Option<String>,
    #[serde(default)]
    pub vence: Option<String>,
    /// Clave técnica del rango, que la DIAN entrega con la autorización. Sin
    /// ella las facturas de esta resolución salen sin CUFE.
    #[serde(default)]
    pub clave_tecnica: Option<String>,
    pub desde: i64,
    pub hasta: i64,
}

pub async fn crear(pool: &Pool, n: &Nueva) -> Resultado<i64> {
    if n.nombre.trim().is_empty() {
        return Err(Error::Regla("la resolución necesita un nombre".into()));
    }
    if n.desde > n.hasta {
        return Err(Error::Regla(format!(
            "rango al revés: [{}..{}]",
            n.desde, n.hasta
        )));
    }
    if n.desde <= 0 {
        return Err(Error::Regla("el rango tiene que empezar en un número positivo".into()));
    }
    Ok(sqlx::query(
        "INSERT INTO resolucion
            (nombre, prefijo, numero, fecha, rige_desde, vence, desde, hasta, clave_tecnica)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(n.nombre.trim())
    .bind(n.prefijo.trim())
    .bind(n.numero.trim())
    .bind(&n.fecha)
    .bind(&n.rige_desde)
    .bind(&n.vence)
    .bind(n.desde)
    .bind(n.hasta)
    .bind(n.clave_tecnica.as_deref().map(str::trim).filter(|c| !c.is_empty()))
    .fetch_one(pool)
    .await?
    .get("id"))
}

pub async fn listar(pool: &Pool, hoy: &str) -> Resultado<Vec<Vista>> {
    let hoy_fecha = chrono::NaiveDate::parse_from_str(hoy, "%Y-%m-%d")
        .map_err(|_| Error::Regla(format!("fecha inválida: {hoy:?}")))?;
    let filas = sqlx::query(
        "SELECT id, nombre, prefijo, numero, fecha, rige_desde, vence, desde, hasta,
                ultimo_emitido, activa, clave_tecnica
           FROM resolucion ORDER BY activa DESC, nombre",
    )
    .fetch_all(pool)
    .await?;

    Ok(filas
        .into_iter()
        .map(|f| {
            let dom = Resolucion {
                id: f.get("id"),
                prefijo: f.get("prefijo"),
                desde: f.get("desde"),
                hasta: f.get("hasta"),
                ultimo_emitido: f.get("ultimo_emitido"),
                vence: f
                    .get::<Option<String>, _>("vence")
                    .and_then(|s| chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()),
                activa: f.get::<i64, _>("activa") == 1,
            };
            Vista {
                quedan: numeracion::quedan(&dom),
                impedimento: numeracion::siguiente(&dom, hoy_fecha).err().map(|e| e.to_string()),
                id: dom.id,
                nombre: f.get("nombre"),
                prefijo: dom.prefijo.clone(),
                numero: f.get("numero"),
                fecha: f.get("fecha"),
                rige_desde: f.get("rige_desde"),
                vence: f.get("vence"),
                clave_tecnica: f.get("clave_tecnica"),
                desde: dom.desde,
                hasta: dom.hasta,
                ultimo_emitido: dom.ultimo_emitido,
                activa: dom.activa,
            }
        })
        .collect())
}

/// Resoluciones a las que les quedan `umbral` números o menos. El aviso que el
/// sistema viejo no daba.
pub async fn por_agotarse(pool: &Pool, hoy: &str, umbral: i64) -> Resultado<Vec<Vista>> {
    Ok(listar(pool, hoy)
        .await?
        .into_iter()
        .filter(|v| v.activa && v.quedan <= umbral)
        .collect())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::prueba_apoyo;

    fn nueva(nombre: &str, desde: i64, hasta: i64) -> Nueva {
        Nueva {
            nombre: nombre.into(),
            prefijo: String::new(),
            numero: "110000135188".into(),
            fecha: Some("2026-01-01".into()),
            rige_desde: None,
            vence: None,
            clave_tecnica: None,
            desde,
            hasta,
        }
    }

    #[tokio::test]
    async fn el_aviso_de_agotamiento_llega_antes_de_agotarse() {
        let pool = prueba_apoyo::base().await;
        // El caso real del corpus: grupo 08, rango 5601..5650.
        let id = crear(&pool, &nueva("Fact. contado", 5601, 5650)).await.unwrap();

        assert!(por_agotarse(&pool, "2026-08-02", 10).await.unwrap().is_empty());

        sqlx::query("UPDATE resolucion SET ultimo_emitido = 5645 WHERE id = ?")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        let avisos = por_agotarse(&pool, "2026-08-02", 10).await.unwrap();
        assert_eq!(avisos.len(), 1);
        assert_eq!(avisos[0].quedan, 5);
        assert!(avisos[0].impedimento.is_none(), "todavía se puede facturar");

        // Y agotada: quedan 0 y hay impedimento con nombre.
        sqlx::query("UPDATE resolucion SET ultimo_emitido = 5650 WHERE id = ?")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        let v = &listar(&pool, "2026-08-02").await.unwrap()[0];
        assert_eq!(v.quedan, 0);
        assert!(v.impedimento.as_ref().unwrap().contains("agotó"), "{v:?}");
    }

    #[tokio::test]
    async fn una_resolucion_vencida_lo_dice_en_la_lista() {
        let pool = prueba_apoyo::base().await;
        let mut n = nueva("Vieja", 1, 100);
        n.vence = Some("2026-01-31".into());
        crear(&pool, &n).await.unwrap();
        let v = &listar(&pool, "2026-08-02").await.unwrap()[0];
        assert!(v.impedimento.as_ref().unwrap().contains("venció"), "{v:?}");
        // Antes del vencimiento, sin impedimento.
        let v = &listar(&pool, "2026-01-15").await.unwrap()[0];
        assert!(v.impedimento.is_none());
    }

    #[tokio::test]
    async fn el_esquema_rechaza_un_rango_al_reves_y_el_codigo_lo_dice_antes() {
        let pool = prueba_apoyo::base().await;
        let e = crear(&pool, &nueva("Mala", 100, 1)).await.unwrap_err();
        assert!(e.to_string().contains("rango"), "salió {e}");
        assert!(crear(&pool, &nueva("Cero", 0, 100)).await.is_err());
        assert!(crear(&pool, &nueva("", 1, 100)).await.is_err());
    }

    #[tokio::test]
    async fn dos_resoluciones_no_comparten_prefijo_y_arranque() {
        let pool = prueba_apoyo::base().await;
        crear(&pool, &nueva("A", 1, 100)).await.unwrap();
        assert!(crear(&pool, &nueva("B", 1, 500)).await.is_err());
        // Pero un arranque distinto con el mismo prefijo sí: es la resolución
        // siguiente del mismo talonario.
        assert!(crear(&pool, &nueva("C", 101, 500)).await.is_ok());
    }
}
