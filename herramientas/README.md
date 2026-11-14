# Herramientas de análisis del corpus heredado

Los guiones que produjeron **todas** las cifras de `ANALISIS.md`. Están aquí
para que cualquiera pueda volver a correrlos y desmentirlas (directiva 2).

    python3 herramientas/leer_dbf.py /mnt/data/inventario_2004/*.DBF   # estructuras
    python3 herramientas/perfilar_corpus.py                            # vivos, conceptos, huérfanos
    python3 herramientas/perfilar_dinero.py                            # aritmética de factura, cartera

`leer_dbf.py` es el lector de referencia contra el que se escribió el de Rust
(`crates/inventario-api/src/dbf.rs`). Que dos lectores independientes den las
mismas cifras es lo que hace creíbles las de `ANALISIS.md`.

Sin dependencias: solo la biblioteca estándar de Python 3.
