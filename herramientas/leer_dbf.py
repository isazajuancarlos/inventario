"""Lector mínimo de DBF (dBase III/IV, FoxPro 2.x, Visual FoxPro).

Sin dependencias. Devuelve estructura y registros. Nada de defectos silenciosos:
si un tipo de campo no se reconoce, se marca CRUDO y se dice.
"""
import struct, datetime, sys, os

TIPOS_CONOCIDOS = set('CNFLDMTIYBGPV0+@O')


def leer_cabecera(path):
    with open(path, 'rb') as f:
        cab = f.read(32)
        if len(cab) < 32:
            raise ValueError(f'{path}: cabecera truncada ({len(cab)} bytes)')
        version = cab[0]
        aa, mm, dd = cab[1], cab[2], cab[3]
        n_reg = struct.unpack('<I', cab[4:8])[0]
        long_cab = struct.unpack('<H', cab[8:10])[0]
        long_reg = struct.unpack('<H', cab[10:12])[0]
        campos = []
        f.seek(32)
        while True:
            d = f.read(32)
            if not d or d[0] in (0x0D, 0x00) or len(d) < 32:
                break
            nombre = d[0:11].split(b'\x00')[0].decode('latin-1').strip()
            tipo = chr(d[11])
            largo = d[16]
            dec = d[17]
            campos.append((nombre, tipo, largo, dec))
        return {
            'path': path,
            'version': version,
            'fecha': f'{1900 + aa if aa < 100 else aa}-{mm:02d}-{dd:02d}',
            'n_reg': n_reg,
            'long_cab': long_cab,
            'long_reg': long_reg,
            'campos': campos,
            'tam_archivo': os.path.getsize(path),
        }


def _conv(tipo, crudo, dec):
    s = crudo.decode('latin-1')
    if tipo == 'C':
        return s.rstrip()
    if tipo in 'NF':
        t = s.strip()
        if not t:
            return None
        try:
            return float(t) if dec else int(t)
        except ValueError:
            return ('NO_NUMERICO', t)
    if tipo == 'D':
        t = s.strip()
        if not t or t == '0' * 8:
            return None
        try:
            return datetime.date(int(t[0:4]), int(t[4:6]), int(t[6:8])).isoformat()
        except ValueError:
            return ('FECHA_INVALIDA', t)
    if tipo == 'L':
        return {'T': True, 'Y': True, 'F': False, 'N': False}.get(s.strip().upper())
    if tipo == 'I':
        return struct.unpack('<i', crudo)[0]
    if tipo in 'YB':
        if tipo == 'Y':
            return struct.unpack('<q', crudo)[0] / 10000.0
        return struct.unpack('<d', crudo)[0]
    if tipo in 'MGP':
        return ('MEMO_REF', s.strip())
    if tipo == 'T':
        j, ms = struct.unpack('<ii', crudo)
        if j == 0:
            return None
        return f'jd{j}+{ms}ms'
    return ('CRUDO', crudo.hex())


def leer(path, limite=None):
    h = leer_cabecera(path)
    campos = h['campos']
    filas = []
    with open(path, 'rb') as f:
        f.seek(h['long_cab'])
        for i in range(h['n_reg']):
            reg = f.read(h['long_reg'])
            if len(reg) < h['long_reg']:
                break
            borrado = reg[0:1] == b'*'
            off = 1
            fila = {'_borrado': borrado}
            for (nom, tipo, largo, dec) in campos:
                fila[nom] = _conv(tipo, reg[off:off + largo], dec)
                off += largo
            filas.append(fila)
            if limite and len(filas) >= limite:
                break
    return h, filas


if __name__ == '__main__':
    for p in sys.argv[1:]:
        h = leer_cabecera(p)
        raros = [c for c in h['campos'] if c[1] not in TIPOS_CONOCIDOS]
        print(f"\n### {os.path.basename(p)}  v0x{h['version']:02x}  reg={h['n_reg']}  "
              f"lreg={h['long_reg']}  ult.actualiz={h['fecha']}  tam={h['tam_archivo']}")
        esperado = h['long_cab'] + h['n_reg'] * h['long_reg']
        if abs(esperado - h['tam_archivo']) > 8:
            print(f"    !! tamaño no cuadra: esperado {esperado}, real {h['tam_archivo']}")
        for (nom, tipo, largo, dec) in h['campos']:
            print(f"    {nom:<12} {tipo} {largo:>3}{('.' + str(dec)) if dec else ''}")
        if raros:
            print(f"    !! tipos no reconocidos: {raros}")
