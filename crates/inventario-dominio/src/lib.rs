//! Las reglas del negocio, sin base de datos y sin HTTP.
//!
//! Todo lo que puede probarse sin abrir un archivo vive aquí: es lo que hace
//! que el costeo, la aritmética de la factura y la numeración se puedan romper
//! a propósito en una prueba y ver caer justo la que los cubre.
//!
//! El diseño responde punto por punto a defectos **medidos** en el sistema que
//! reemplaza; están documentados en `ANALISIS.md`, en la raíz del repositorio.

pub mod cartera;
pub mod costeo;
pub mod cufe;
pub mod dinero;
pub mod factura;
pub mod gtin;
pub mod kardex;
pub mod letras;
pub mod nit;
pub mod numeracion;
pub mod retiro;
pub mod vnr;

pub use dinero::{Cantidad, Centavos};
