// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Juan Carlos Isaza
//
// sugerir-de-opcion — al elegir en un `<select>`, copiar un dato de la opción
// elegida a otro campo de la MISMA fila, y solo si ese campo sigue intacto.
//
// Nace el 2026-08-12 para cerrar el `data-precio` de las líneas de factura, que
// llevaba desde la migración siendo un atributo MUERTO: el comentario del
// servidor prometía «viaja en el option para poder sugerirlo» y nadie lo leía.
// O se usa o se quita; esto es usarlo.
//
// **Sugerir, no imponer**: el precio que manda sigue siendo el que se escriba,
// así que no pisa un valor que quien factura ya haya tocado.
//
// VIVE AQUÍ Y NO EN `boi` A PROPÓSITO (directiva 9): a la librería sube lo que YA
// se repitió en un segundo proyecto, nunca por previsión. Hoy lo pide uno.
//
//   <select data-sugerir="precio" data-sugerir-a="input[name=precio]">
//     <option value="7" data-precio="12000">…</option>
//   </select>
import { registrar } from "../boi.js";

registrar("sugerir-de-opcion", "select[data-sugerir][data-sugerir-a]", (select) => {
  const cual = select.dataset.sugerir;
  const selector = select.dataset.sugerirA;
  // El destino se busca dentro de la FILA, no del documento: en una tabla de
  // líneas hay uno por fila, y un `querySelector` global escribiría siempre en
  // el primero.
  const fila = select.closest("tr") || select.closest("form") || document;
  const destino = fila.querySelector(selector);
  if (!destino) return;

  destino.addEventListener("keydown", () => {
    destino.dataset.tocado = "";
  });

  select.addEventListener("change", () => {
    const opcion = select.selectedOptions[0];
    if (!opcion) return;
    const valor = opcion.dataset[cual];
    if (valor === undefined) return;
    if ("tocado" in destino.dataset) return;             // lo escrito no se pisa
    if (destino.value !== "" && destino.value !== "0") return;
    destino.value = valor;
  });
});
