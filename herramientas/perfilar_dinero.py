import sys, os, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from dbf import leer
BASE = '/mnt/data/inventario_2004'
def c(n):
    h, f = leer(os.path.join(BASE, n))
    return [x for x in f if not x['_borrado']], f

fac, fac_all = c('facturas.DBF')
det, _ = c('fac_articulo.DBF')
art, _ = c('ARTICULOS.DBF')
cli, _ = c('CLIENTES.DBF')
mov, _ = c('mov_inv.DBF')
enc, enc_all = c('ENCRECIBO.DBF')
dre, dre_all = c('DETRECIBO.DBF')
cat, cat_all = c('CATEGORIAS.DBF')
vc, _ = c('valorconvenio.dbf')
conv, _ = c('convenios.DBF')

print('=== FACTURAS por año / tipo ===')
por = collections.Counter((r['FECHAFAC'][:4] if r['FECHAFAC'] else '????', r['TIPO'].strip()) for r in fac)
for k in sorted(por): print(f'  {k}  n={por[k]}')

print('\n=== MOV_INV por mes ===')
pm = collections.Counter(r['FECHA'][:7] if r['FECHA'] else '???' for r in mov)
for k in sorted(pm): print(f'  {k} n={pm[k]}')

print('\n=== Dinero: IVA, retefuente, descuento realmente usados ===')
def stats(rows, campo):
    v = [r[campo] for r in rows if isinstance(r.get(campo), (int, float))]
    nz = [x for x in v if x]
    return f'n={len(v)} no_cero={len(nz)} min={min(v) if v else 0} max={max(v) if v else 0} suma={sum(v):,.2f}'
for campo in ['VALORFAC', 'VALORIVA', 'RTEFTE', 'DESCTO']:
    print(f'  facturas.{campo:<10} {stats(fac, campo)}')
print(f"  neg VALORFAC: {len([r for r in fac if isinstance(r['VALORFAC'],(int,float)) and r['VALORFAC']<0])}")

print('\n=== ¿cuadra la cabecera con el detalle? (directiva 2) ===')
sumdet = collections.defaultdict(float)
for d in det:
    if isinstance(d['CANTIDAD'], (int, float)) and isinstance(d['VALOR'], (int, float)):
        sumdet[d['NROFAC']] += d['CANTIDAD'] * d['VALOR']
desc = igual = sin = 0
peor = []
for f in fac:
    n = f['NROFAC']
    if n not in sumdet:
        sin += 1; continue
    cab = f['VALORFAC'] or 0
    dif = abs(cab - sumdet[n])
    if dif < 1: igual += 1
    else:
        desc += 1; peor.append((dif, n, cab, sumdet[n]))
peor.sort(reverse=True)
print(f'  cuadran={igual}  descuadran={desc}  sin detalle={sin}   (de {len(fac)} facturas)')
for d, n, a, b in peor[:6]: print(f'     fac {n}: cabecera={a:,.2f} detalle={b:,.2f} dif={d:,.2f}')

print('\n=== ARTICULOS: muestra y campos vivos ===')
for r in art[:4]:
    print(f"  cod={r['COD_ARTI']!r} barr={r['COD_BARR']!r} d1={r['DESCRI1']!r} pres={r['PRESENTA']!r}")
    print(f"      inv={r['INV_ACTUAL']} costo={r['VAL_COSTO']} vta={r['VAL_VTA']} iva={r['IVA']} cat={r['COD_CATEGO']!r} min={r['PTO_MIN']} max={r['PTO_MAX']}")
for campo in ['INV_ACTUAL', 'VAL_COSTO', 'VAL_VTA', 'IVA', 'PTO_MIN', 'PTO_MAX', 'SALDOINI', 'COSTO_INI', 'COSTO_SAL']:
    v = [r[campo] for r in art if isinstance(r.get(campo), (int, float))]
    nz = [x for x in v if x]
    print(f'  {campo:<12} no_cero={len(nz):>4}/{len(v)}  max={max(v) if v else 0}')
print(f"  IVA distintos: {dict(collections.Counter(r['IVA'] for r in art))}")
print(f"  inventario negativo: {[ (r['COD_ARTI'].strip(), r['INV_ACTUAL']) for r in art if isinstance(r['INV_ACTUAL'],(int,float)) and r['INV_ACTUAL']<0 ][:10]}")
print(f"  COD_ARTI duplicados: {[k for k,v in collections.Counter(r['COD_ARTI'].strip() for r in art).items() if v>1]}")
print(f"  COD_BARR vacíos: {len([r for r in art if not r['COD_BARR'].strip()])}/{len(art)}")

print('\n=== CATEGORIAS (6 vivas de 12) ===')
for r in cat_all: print(f"   {'X' if r['_borrado'] else ' '} {r['COD_CATEGO']!r} {r['NOM_CATEGO']!r} sugerido={r['SUGERIDO']}")
uso = collections.Counter(r['COD_CATEGO'].strip() for r in art)
print(f'  uso en articulos: {dict(uso)}')

print('\n=== RECIBOS: la cabecera se borró, ¿y el detalle? ===')
encnum = {r['NROREC'] for r in enc}
encnum_all = {r['NROREC'] for r in enc_all}
h = [d for d in dre if d['NROREC'] not in encnum]
h2 = [d for d in dre if d['NROREC'] not in encnum_all]
print(f'  DETRECIBO vivos={len(dre)}  sin cabecera VIVA={len(h)}  sin cabecera NI BORRADA={len(h2)}')
print(f'  suma abonos en DETRECIBO = {sum(d["VALOR"] or 0 for d in dre):,.2f}')
print(f'  formas de pago no cero: cheque={len([d for d in dre if d["VALCHEQUE"]])} efe={len([d for d in dre if d["VALOREFE"]])} tarC={len([d for d in dre if d["VALORTARC"]])} tarD={len([d for d in dre if d["VALORTARD"]])} nota={len([d for d in dre if d["VALORNOTA"]])}')

print('\n=== CARTERA: facturas a crédito vs abonos ===')
abon = collections.defaultdict(float)
for d in dre: abon[d['NROFAC']] += d['VALOR'] or 0
credito = [f for f in fac if f['TIPO'].strip() == 'C']
pend = [f for f in fac if f['ESTADOFAC'].strip() == 'P']
print(f'  facturas TIPO=C: {len(credito)}  ESTADOFAC=P (pendiente): {len(pend)}  N (anulada): {len([f for f in fac if f["ESTADOFAC"].strip()=="N"])}')
saldo = sum((f['VALORFAC'] or 0) - abon.get(f['NROFAC'], 0) for f in pend)
print(f'  saldo teórico de cartera pendiente = {saldo:,.2f}')

print('\n=== CONVENIOS y lista de precios ===')
for r in conv: print(f"   {r['COD_CONVE']!r} {r['NOM_CONVE']!r}")
print(f'  valorconvenio vivos={len(vc)} convenios distintos={dict(collections.Counter(r["COD_CONVE"].strip() for r in vc))}')
usoconv = collections.Counter(r['COD_CONVE'].strip() for r in cli)
print(f'  clientes por convenio: {dict(usoconv)}')

print('\n=== CLIENTES: calidad ===')
print(f"  sin nombre: {len([r for r in cli if not r['NOM_CLIEN'].strip()])}")
print(f"  sin NIT: {len([r for r in cli if not r['NIT_CLIEN'].strip()])}")
print(f"  PAGO: {dict(collections.Counter(r['PAGO'].strip() for r in cli))}")
print(f"  ZONA vacía: {len([r for r in cli if not r['ZONA'].strip()])}  CIUDAD vacía: {len([r for r in cli if not r['CIUDAD'].strip()])}")
act = {f['IDCLIENTE'].strip() for f in fac if f['FECHAFAC'] and f['FECHAFAC'] >= '2018-01-01'}
print(f'  clientes con factura desde 2018: {len(act)} de {len(cli)}')
