"""Perfila las tablas vivas: rangos de fecha, cardinalidades, integridad referencial."""
import sys, os, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from dbf import leer

BASE = "/mnt/data/inventario_2004"

def carga(nombre):
    h, filas = leer(os.path.join(BASE, nombre))
    vivos = [f for f in filas if not f['_borrado']]
    return h, filas, vivos

TABLAS = ['ARTICULOS.DBF', 'CLIENTES.DBF', 'facturas.DBF', 'fac_articulo.DBF',
          'mov_inv.DBF', 'CONCEPTOS.DBF', 'ENCRECIBO.DBF', 'DETRECIBO.DBF',
          'valorconvenio.dbf', 'convenios.DBF', 'CATEGORIAS.DBF', 'vendedor.DBF',
          'PROVEEDOR.DBF', 'CONFIFAC.DBF', 'CONTROL.DBF', 'USUARIOS.DBF',
          'PERMISOS.DBF', 'pedidos.DBF', 'DESPACHOS.DBF', 'zonas.DBF',
          'BARRIOS.DBF', 'ciudades.DBF', 'ENOTACRED.DBF', 'dnotacred.DBF',
          'bancos.DBF', 'confirec.dbf', 'MONTOESC.DBF']

datos = {}
print('=' * 78)
print(f"{'tabla':<22}{'total':>8}{'vivos':>8}{'borrados':>10}")
print('=' * 78)
for t in TABLAS:
    try:
        h, filas, vivos = carga(t)
        datos[t] = vivos
        print(f'{t:<22}{len(filas):>8}{len(vivos):>8}{len(filas)-len(vivos):>10}')
    except Exception as e:
        print(f'{t:<22}  ERROR {e}')

def rango_fecha(rows, campo):
    v = sorted(r[campo] for r in rows if isinstance(r.get(campo), str) and r[campo])
    return (v[0], v[-1], len(v)) if v else (None, None, 0)

print('\n=== RANGOS DE FECHA (lo que dice si la tabla está viva) ===')
for t, campo in [('facturas.DBF', 'FECHAFAC'), ('mov_inv.DBF', 'FECHA'),
                 ('ENCRECIBO.DBF', 'FECHAABO'), ('CLIENTES.DBF', 'FECHAREG'),
                 ('pedidos.DBF', 'FECHA'), ('DESPACHOS.DBF', 'FEC_DESPA'),
                 ('ARTICULOS.DBF', 'FECHAREG'), ('ENOTACRED.DBF', 'FECHA')]:
    if t in datos:
        a, b, n = rango_fecha(datos[t], campo)
        print(f'  {t:<20} {campo:<12} {a} .. {b}   ({n} con fecha)')

print('\n=== CONCEPTOS de movimiento (el corazón del kardex) ===')
for c in sorted(datos.get('CONCEPTOS.DBF', []), key=lambda r: r['COD_CONCEP']):
    print(f"  {c['COD_CONCEP']:<6} {c['NOM_CONCEP']:<36} signo={c['SIGNO']!r:<5} "
          f"prov={c['PROVEEDOR']!r} ini={c['INICIAL']!r} gastos={c['GASTOS']!r} cli={c['CLIENTE']!r}")

print('\n=== uso real de cada concepto en mov_inv ===')
uso = collections.Counter(r['COD_CONCEP'].strip() for r in datos.get('mov_inv.DBF', []))
nom = {c['COD_CONCEP'].strip(): (c['NOM_CONCEP'], c['SIGNO']) for c in datos.get('CONCEPTOS.DBF', [])}
for cod, n in uso.most_common():
    d = nom.get(cod, ('<<NO EXISTE EN CONCEPTOS>>', '?'))
    print(f'  {cod:<6} n={n:<6} {d[1]!r:<5} {d[0]}')

print('\n=== CONTROL (parámetros de la instalación) ===')
for k, v in (datos.get('CONTROL.DBF') or [{}])[0].items():
    print(f'  {k:<14} {v!r}')

print('\n=== CONFIFAC (resoluciones DIAN) ===')
for c in datos.get('CONFIFAC.DBF', []):
    print(f"  grupo={c['GRUPOFAC']!r} pref={c['PREFIJO']!r} [{c['NROMINFAC']}..{c['NROMAXFAC']}] "
          f"ult={c['ULT_FACTU']} reso={c['NRORESO']!r} fecha={c['FECHARESO']} nom={c['NOMGRUFAC']!r}")

print('\n=== INTEGRIDAD REFERENCIAL (huérfanos = deuda de migración) ===')
arts = {r['COD_ARTI'].strip() for r in datos.get('ARTICULOS.DBF', [])}
clis = {r['IDCLIENTE'].strip() for r in datos.get('CLIENTES.DBF', [])}
facs = {r['NROFAC'] for r in datos.get('facturas.DBF', [])}

def huerf(tabla, campo, universo, etiqueta):
    rows = datos.get(tabla, [])
    faltan = collections.Counter()
    for r in rows:
        v = r[campo]
        v = v.strip() if isinstance(v, str) else v
        if v not in universo:
            faltan[v] += 1
    print(f'  {tabla}.{campo} -> {etiqueta}: {sum(faltan.values())}/{len(rows)} huérfanos, '
          f'{len(faltan)} claves distintas')
    for v, n in faltan.most_common(5):
        print(f'        {v!r} x{n}')

huerf('fac_articulo.DBF', 'COD_ARTI', arts, 'ARTICULOS')
huerf('fac_articulo.DBF', 'NROFAC', facs, 'facturas')
huerf('mov_inv.DBF', 'COD_ARTI', arts, 'ARTICULOS')
huerf('facturas.DBF', 'IDCLIENTE', clis, 'CLIENTES')
huerf('ENCRECIBO.DBF', 'IDCLIENTE', clis, 'CLIENTES')
huerf('valorconvenio.dbf', 'COD_ARTI', arts, 'ARTICULOS')

print('\n=== IDCLIENTE: ¿qué forma tiene? ===')
for r in datos.get('CLIENTES.DBF', [])[:3]:
    print(f"  nit={r['NIT_CLIEN']!r} id={r['IDCLIENTE']!r} nom={r['NOM_CLIEN']!r}")
largos = collections.Counter(len(r['IDCLIENTE'].strip()) for r in datos.get('CLIENTES.DBF', []))
print(f'  largos de IDCLIENTE: {dict(largos)}')
nits = collections.Counter(r['NIT_CLIEN'].strip() for r in datos.get('CLIENTES.DBF', []))
dup = {k: v for k, v in nits.items() if v > 1}
print(f'  NIT duplicados: {len(dup)} (ej: {list(dup.items())[:5]})')
ids = collections.Counter(r['IDCLIENTE'].strip() for r in datos.get('CLIENTES.DBF', []))
print(f'  IDCLIENTE duplicados: {len([k for k,v in ids.items() if v>1])}')

print('\n=== NROFAC: ¿único? ¿colisiona entre prefijos? ===')
f = datos.get('facturas.DBF', [])
pref = collections.Counter((r['PREFIJO'].strip(), r['TIPO'].strip()) for r in f)
print(f'  (prefijo,tipo): {dict(pref)}')
nn = collections.Counter(r['NROFAC'] for r in f)
print(f'  NROFAC repetidos: {len([k for k,v in nn.items() if v>1])}')
nnp = collections.Counter((r['PREFIJO'].strip(), r['NROFAC']) for r in f)
print(f'  (prefijo,NROFAC) repetidos: {len([k for k,v in nnp.items() if v>1])}')
est = collections.Counter(r['ESTADOFAC'].strip() for r in f)
print(f'  ESTADOFAC: {dict(est)}')
dest = collections.Counter(r['DESTINO'].strip() for r in f)
print(f'  DESTINO: {dict(dest)}')
