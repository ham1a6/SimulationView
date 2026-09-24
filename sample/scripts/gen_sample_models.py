#!/usr/bin/env python3
"""サンプル用の3Dモデル(glTF 2.0のGLB)を生成する。標準ライブラリだけで動く。

    python sample/scripts/gen_sample_models.py

`sample/sim_frontend/assets/models/`に、種別ごとの簡易な低ポリゴンモデルを書き出す
(aircraft / helicopter / ship / vehicle / missile)。sim3dviewライブラリの3Dモデル表示(`terrain::models`、
DETAILED_DESIGN.md 6.13節)の動作確認用で、見た目は模式的なもの(テクスチャなし、マテリアルの基本色だけ)。
自分のモデルを使うときは、同じ規約のGLBを`ModelsState::set_source`で登録すればよい。

規約: 単位はメートル、実寸。モデルは「機体座標」(x=右、y=前、z=上)で作り、書き出すときにglTFの座標
(+Y上・+Z前・+X左)へ回す。原点は基準点(航空機・ヘリ・ミサイルは中心、艦船は水線の中央、車両は接地面の中央)。
"""
import json
import math
import os
import struct

OUT_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'sim_frontend', 'assets', 'models')


def sub(a, b):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def normalize(v):
    n = math.sqrt(v[0] ** 2 + v[1] ** 2 + v[2] ** 2)
    return (v[0] / n, v[1] / n, v[2] / n) if n > 1e-12 else (0.0, 0.0, 1.0)


class Mesh:
    """色ごとにプリミティブを持つメッシュ。頂点は機体座標(x右・y前・z上)、三角形は外から見て反時計回り。"""

    def __init__(self, name):
        self.name = name
        self.prims = {}  # 色 -> {'pos': [], 'nrm': [], 'idx': []}

    def _prim(self, color):
        return self.prims.setdefault(color, {'pos': [], 'nrm': [], 'idx': []})

    def add(self, color, verts, tris, smooth):
        prim = self._prim(color)
        tris = [t for t in tris if math.sqrt(sum(c * c for c in cross(sub(verts[t[1]], verts[t[0]]), sub(verts[t[2]], verts[t[0]])))) > 1e-9]
        if smooth:
            normals = [[0.0, 0.0, 0.0] for _ in verts]
            for a, b, c in tris:
                n = cross(sub(verts[b], verts[a]), sub(verts[c], verts[a]))  # 面積で重みが付く
                for i in (a, b, c):
                    for k in range(3):
                        normals[i][k] += n[k]
            base = len(prim['pos'])
            prim['pos'] += verts
            prim['nrm'] += [normalize(n) for n in normals]
            prim['idx'] += [base + i for t in tris for i in t]
        else:
            for a, b, c in tris:
                n = normalize(cross(sub(verts[b], verts[a]), sub(verts[c], verts[a])))
                base = len(prim['pos'])
                prim['pos'] += [verts[a], verts[b], verts[c]]
                prim['nrm'] += [n, n, n]
                prim['idx'] += [base, base + 1, base + 2]

    # --- 形 ---

    def prism(self, color, polygon, plane, w0, w1):
        """2Dの凸多角形`polygon`(平面`plane`の(u,v)座標)を、垂直な軸に沿って`w0`〜`w1`まで押し出した柱。
        plane: 'xy'(zへ押し出す)・'yz'(xへ)・'zx'(yへ)。ふつうの向きで書けば、回りは自動で直す。"""
        area = sum(polygon[i][0] * polygon[(i + 1) % len(polygon)][1] - polygon[(i + 1) % len(polygon)][0] * polygon[i][1]
                   for i in range(len(polygon)))
        if area < 0:
            polygon = polygon[::-1]

        def to_body(u, v, w):
            return {'xy': (u, v, w), 'yz': (w, u, v), 'zx': (v, w, u)}[plane]

        n = len(polygon)
        verts = [to_body(u, v, w0) for u, v in polygon] + [to_body(u, v, w1) for u, v in polygon]
        tris = []
        for i in range(1, n - 1):
            tris.append((0, i + 1, i))          # 下の面(-w向き)
            tris.append((n, n + i, n + i + 1))  # 上の面(+w向き)
        for i in range(n):
            j = (i + 1) % n
            tris += [(i, j, n + j), (i, n + j, n + i)]  # 側面(外向き)
        self.add(color, verts, tris, smooth=False)

    def box(self, color, center, size):
        cx, cy, cz = center
        hx, hy, hz = size[0] / 2, size[1] / 2, size[2] / 2
        self.prism(color, [(cx - hx, cy - hy), (cx + hx, cy - hy), (cx + hx, cy + hy), (cx - hx, cy + hy)], 'xy', cz - hz, cz + hz)

    def lathe(self, color, profile, axis='y', center=(0.0, 0.0, 0.0), segments=16, sx=1.0, sz=1.0):
        """軸まわりの回転体。`profile`は(軸方向の位置, 半径)の並び(位置の小さい方から)。半径0の点が端の閉じた先端になる。
        隣り合う点は頂点を共有してなめらかな陰影になる。円筒の端の面を平らにしたいときは、同じ位置の点を2つ並べる。"""
        rings = []
        for a, r in profile:
            ring = []
            for j in range(segments):
                t = 2 * math.pi * j / segments
                p, q = r * math.cos(t) * sx, r * math.sin(t) * sz
                x, y, z = {'y': (p, a, q), 'z': (q, p, a), 'x': (a, q, p)}[axis]
                ring.append((x + center[0], y + center[1], z + center[2]))
            rings.append(ring)
        verts = [v for ring in rings for v in ring]
        tris = []
        for i in range(len(rings) - 1):
            for j in range(segments):
                k = (j + 1) % segments
                a, b, c, d = i * segments + j, (i + 1) * segments + j, (i + 1) * segments + k, i * segments + k
                tris += [(a, b, c), (a, c, d)]
        self.add(color, verts, tris, smooth=True)

    def ellipsoid(self, color, center, radii, segments=16, rings=8):
        profile = [(radii[1] * math.cos(math.pi * i / rings), math.sin(math.pi * i / rings)) for i in range(rings + 1)]
        # 軸方向を前後(y)にとった回転体を、x・zの半径で横に広げる。位置の小さい方から並べる(y=-ry→+ry)。
        profile = profile[::-1]
        self.lathe(color, profile, 'y', center, segments, sx=radii[0], sz=radii[2])

    # --- 書き出し ---

    def write_glb(self, path):
        buffer = bytearray()
        views, accessors, primitives, materials = [], [], [], []
        # 機体座標(x右・y前・z上)→glTF(+X左・+Y上・+Z前): (x,y,z)→(-x,z,y)。回転なので三角形の回りは変わらない。
        to_gltf = lambda v: (-v[0], v[2], v[1])

        def add_view(data, target):
            while len(buffer) % 4:
                buffer.append(0)
            views.append({'buffer': 0, 'byteOffset': len(buffer), 'byteLength': len(data), 'target': target})
            buffer.extend(data)
            return len(views) - 1

        for color, prim in self.prims.items():
            pos = [to_gltf(p) for p in prim['pos']]
            nrm = [to_gltf(n) for n in prim['nrm']]
            pos_view = add_view(b''.join(struct.pack('<3f', *p) for p in pos), 34962)
            nrm_view = add_view(b''.join(struct.pack('<3f', *n) for n in nrm), 34962)
            idx_view = add_view(b''.join(struct.pack('<I', i) for i in prim['idx']), 34963)
            lo = [min(p[k] for p in pos) for k in range(3)]
            hi = [max(p[k] for p in pos) for k in range(3)]
            base = len(accessors)
            accessors += [
                {'bufferView': pos_view, 'componentType': 5126, 'count': len(pos), 'type': 'VEC3', 'min': lo, 'max': hi},
                {'bufferView': nrm_view, 'componentType': 5126, 'count': len(nrm), 'type': 'VEC3'},
                {'bufferView': idx_view, 'componentType': 5125, 'count': len(prim['idx']), 'type': 'SCALAR'},
            ]
            materials.append({'name': 'rgb_%d_%d_%d' % tuple(int(c * 255) for c in color),
                              'pbrMetallicRoughness': {'baseColorFactor': [*color, 1.0], 'metallicFactor': 0.0, 'roughnessFactor': 0.8}})
            primitives.append({'attributes': {'POSITION': base, 'NORMAL': base + 1}, 'indices': base + 2, 'material': len(materials) - 1})
        doc = {
            'asset': {'version': '2.0', 'generator': 'sim3dview scripts/gen_sample_models.py'},
            'scene': 0, 'scenes': [{'nodes': [0]}],
            'nodes': [{'name': self.name, 'mesh': 0}],
            'meshes': [{'name': self.name, 'primitives': primitives}],
            'materials': materials, 'buffers': [{'byteLength': len(buffer)}],
            'bufferViews': views, 'accessors': accessors,
        }
        json_bytes = json.dumps(doc, separators=(',', ':')).encode()
        json_bytes += b' ' * (-len(json_bytes) % 4)
        buffer.extend(b'\0' * (-len(buffer) % 4))
        total = 12 + 8 + len(json_bytes) + 8 + len(buffer)
        with open(path, 'wb') as f:
            f.write(b'glTF' + struct.pack('<II', 2, total))
            f.write(struct.pack('<I', len(json_bytes)) + b'JSON' + json_bytes)
            f.write(struct.pack('<I', len(buffer)) + b'BIN\0' + bytes(buffer))

    def bounds(self):
        pts = [p for prim in self.prims.values() for p in prim['pos']]
        return [min(p[k] for p in pts) for k in range(3)], [max(p[k] for p in pts) for k in range(3)], len(pts)


def mirrored(points):
    """右側の点の並び(前から後ろ)から、左右対称の多角形の頂点列を作る(中心線上の点は重ねない)。"""
    return points + [(-x, y) for x, y in reversed(points) if x != 0]


# ---- モデル ----

GRAY, DARK_GRAY, CANOPY = (0.55, 0.58, 0.62), (0.42, 0.45, 0.5), (0.12, 0.18, 0.28)


def aircraft():
    """戦闘機風(全長約15m・翼幅約10m)。原点は胴体の中央。"""
    m = Mesh('aircraft')
    m.lathe(GRAY, [(-7.0, 0.0), (-7.0, 0.35), (-6.0, 0.85), (-3.0, 1.0), (2.0, 0.9), (4.5, 0.55), (6.5, 0.22), (7.6, 0.0)], 'y', segments=14)
    m.ellipsoid(CANOPY, (0.0, 2.6, 0.7), (0.42, 1.5, 0.42), segments=10, rings=6)
    right_wing = [(0.8, 1.6), (5.0, -2.6), (5.0, -4.2), (0.8, -4.6)]
    m.prism(DARK_GRAY, mirrored(right_wing), 'xy', -0.1, 0.1)
    right_tail = [(0.8, -5.0), (2.9, -6.7), (2.9, -7.4), (0.8, -7.1)]
    m.prism(DARK_GRAY, mirrored(right_tail), 'xy', -0.08, 0.08)
    m.prism(DARK_GRAY, [(-3.4, 0.8), (-6.5, 3.2), (-7.4, 3.2), (-6.6, 0.8)], 'yz', -0.08, 0.08)
    return m


OLIVE, DARK = (0.34, 0.4, 0.3), (0.16, 0.17, 0.18)


def helicopter():
    """ヘリコプター(全長約13m・ローター直径約10.5m)。原点は胴体の中央。"""
    m = Mesh('helicopter')
    m.ellipsoid(OLIVE, (0.0, 0.4, 0.0), (1.1, 2.4, 1.2))
    m.lathe(OLIVE, [(-6.6, 0.18), (-1.6, 0.42)], 'y', center=(0.0, 0.0, 0.35), segments=10)
    m.prism(OLIVE, [(-6.0, 0.4), (-6.7, 1.9), (-7.1, 1.9), (-6.7, 0.4)], 'yz', -0.07, 0.07)
    m.lathe(DARK, [(-0.05, 0.0), (-0.05, 0.6), (0.05, 0.6), (0.05, 0.0)], 'x', center=(0.25, -6.8, 1.3), segments=10)
    m.lathe(DARK, [(1.0, 0.0), (1.0, 0.16), (1.9, 0.16), (1.9, 0.0)], 'z', segments=8)
    m.box(DARK, (0.0, 0.0, 1.95), (10.5, 0.35, 0.06))
    m.box(DARK, (0.0, 0.0, 1.95), (0.35, 10.5, 0.06))
    for side in (-1, 1):
        m.box(DARK, (side * 1.05, 0.3, -1.45), (0.12, 3.6, 0.12))
        m.box(DARK, (side * 0.95, 1.2, -1.0), (0.1, 0.1, 0.9))
        m.box(DARK, (side * 0.95, -0.6, -1.0), (0.1, 0.1, 0.9))
    return m


HULL_UP, HULL_LOW, SUPER, FUNNEL = (0.5, 0.53, 0.57), (0.32, 0.12, 0.1), (0.62, 0.64, 0.67), (0.35, 0.37, 0.4)


def ship():
    """駆逐艦風(全長約128m・幅16m)。原点は水線の中央(水線より4m下まで船体がある)。"""
    m = Mesh('ship')
    right = [(0.0, 64.0), (5.0, 50.0), (8.0, 30.0), (8.0, -55.0), (7.5, -63.0), (0.0, -63.0)]
    m.prism(HULL_UP, mirrored(right), 'xy', 0.0, 5.0)
    lower = [(0.0, 60.0), (3.5, 48.0), (6.0, 30.0), (6.0, -55.0), (5.5, -62.0), (0.0, -62.0)]
    m.prism(HULL_LOW, mirrored(lower), 'xy', -4.0, 0.0)
    m.box(SUPER, (0.0, 8.0, 9.5), (10.0, 24.0, 9.0))
    m.box(SUPER, (0.0, 10.0, 17.0), (6.0, 9.0, 6.0))
    m.box(DARK, (0.0, 10.0, 25.0), (0.6, 0.6, 10.0))
    m.box(FUNNEL, (0.0, -12.0, 14.0), (5.0, 8.0, 8.0))
    m.box(SUPER, (0.0, -30.0, 8.0), (7.0, 12.0, 6.0))
    m.box(HULL_LOW, (0.0, 38.0, 6.0), (3.6, 3.6, 2.0))
    m.lathe(DARK, [(38.0, 0.0), (38.0, 0.5), (46.0, 0.5), (46.0, 0.0)], 'y', center=(0.0, 0.0, 6.6), segments=10)
    return m


TAN, TRACK = (0.4, 0.42, 0.3), (0.13, 0.13, 0.13)


def vehicle():
    """戦車風の装軌車両(全長約7.5m・幅3.6m)。原点は接地面の中央。"""
    m = Mesh('vehicle')
    m.box(TAN, (0.0, 0.0, 1.05), (2.8, 6.6, 1.1))
    for side in (-1, 1):
        m.box(TRACK, (side * 1.6, 0.0, 0.5), (0.7, 7.4, 1.0))
    m.lathe(TAN, [(1.6, 0.0), (1.6, 1.45), (2.4, 1.45), (2.4, 0.0)], 'z', center=(0.0, -0.4, 0.0), segments=14)
    m.lathe(DARK, [(0.8, 0.0), (0.8, 0.15), (5.4, 0.15), (5.4, 0.0)], 'y', center=(0.0, 0.0, 2.05), segments=8)
    return m


WHITE, RED = (0.86, 0.86, 0.88), (0.6, 0.12, 0.1)


def missile():
    """ミサイル(全長約5m・直径0.4m)。原点は中央。"""
    m = Mesh('missile')
    m.lathe(WHITE, [(-2.5, 0.0), (-2.5, 0.2), (1.4, 0.2)], 'y', segments=12)
    m.lathe(RED, [(1.4, 0.2), (2.0, 0.15), (2.5, 0.0)], 'y', segments=12)
    m.box(DARK, (0.0, -2.1, 0.0), (1.4, 0.55, 0.03))
    m.box(DARK, (0.0, -2.1, 0.0), (0.03, 0.55, 1.4))
    return m


def main():
    os.makedirs(OUT_DIR, exist_ok=True)
    for build in (aircraft, helicopter, ship, vehicle, missile):
        mesh = build()
        path = os.path.normpath(os.path.join(OUT_DIR, mesh.name + '.glb'))
        mesh.write_glb(path)
        lo, hi, count = mesh.bounds()
        size = [round(hi[k] - lo[k], 1) for k in range(3)]
        print('%-10s 頂点%5d  幅x前後y高さz = %s m  (%d bytes)  %s' % (mesh.name, count, size, os.path.getsize(path), path))


if __name__ == '__main__':
    main()
