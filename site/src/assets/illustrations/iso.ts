/**
 * Isometric geometry for the Dockside illustration system.
 *
 * Every piece is projected with the same axes as the 32×32 crate mark
 * (`web/admin/src/components/brand/cratebase-mark.tsx`): a unit cube edge
 * runs 11 across and 5.8 down on screen, and a vertical edge is 10.4 — the
 * mark's hexagon exactly, scaled by `U`. Cubes drawn here therefore *are* the
 * mark, with faces filled.
 *
 * World axes: +x runs right-and-down on screen, +y runs left-and-down, +z is
 * up. The viewer stands at +x +y +z, so a box shows its top, its y+sy face
 * (screen left) and its x+sx face (screen right).
 *
 * All helpers return SVG path `d` strings; pieces compose them in Astro
 * frontmatter at build time, so the output is static SVG with no runtime.
 */

export type V3 = readonly [number, number, number];
export type Pt = readonly [number, number];

/** Screen pixels per world unit along a cube edge's horizontal run. */
const U = 20;
const W = U; // horizontal run of a unit edge
const H = (U * 5.8) / 11; // vertical drop of a unit edge (mark: 5.8 / 11)
const V = (U * 10.4) / 11; // screen height of a unit vertical edge (mark: 10.4 / 11)

export function project([x, y, z]: V3): Pt {
  return [(x - y) * W, (x + y) * H - z * V];
}

const f = (n: number): string => {
  const r = Math.round(n * 100) / 100;
  return Object.is(r, -0) ? "0" : String(r);
};

function d(points: readonly Pt[], close: boolean): string {
  let out = "";
  for (let i = 0; i < points.length; i++) {
    const [x, y] = points[i];
    out += `${i === 0 ? "M" : "L"}${f(x)} ${f(y)}`;
  }
  return close ? out + "Z" : out;
}

/** Closed polygon through world points. */
export function poly(points: readonly V3[]): string {
  return d(points.map(project), true);
}

/** Open polyline through world points. */
export function polyline(points: readonly V3[]): string {
  return d(points.map(project), false);
}

export function line(a: V3, b: V3): string {
  return polyline([a, b]);
}

export type Plane = "xy" | "xz" | "yz";

function onPlane(center: V3, plane: Plane, u: number, v: number): V3 {
  const [cx, cy, cz] = center;
  switch (plane) {
    case "xy":
      return [cx + u, cy + v, cz];
    case "xz":
      return [cx + u, cy, cz + v];
    case "yz":
      return [cx, cy + u, cz + v];
  }
}

/** Cubic bezier through world-space control points, sampled to `segments` points. */
export function bezier(p0: V3, p1: V3, p2: V3, p3: V3, segments = 24): V3[] {
  const pts: V3[] = [];
  for (let i = 0; i <= segments; i++) {
    const t = i / segments;
    const s = 1 - t;
    const a = s * s * s;
    const b = 3 * s * s * t;
    const c = 3 * s * t * t;
    const e = t * t * t;
    pts.push([
      a * p0[0] + b * p1[0] + c * p2[0] + e * p3[0],
      a * p0[1] + b * p1[1] + c * p2[1] + e * p3[1],
      a * p0[2] + b * p1[2] + c * p2[2] + e * p3[2],
    ]);
  }
  return pts;
}

/**
 * Arc of a circle lying in `plane`, from angle `from` to `to` (radians,
 * counter-clockwise in the plane's (u, v) basis). Sampled, so it projects to
 * the correct ellipse in any plane.
 */
export function arc(
  center: V3,
  r: number,
  plane: Plane,
  from: number,
  to: number,
  segments = 24,
): string {
  const pts: V3[] = [];
  for (let i = 0; i <= segments; i++) {
    const t = from + ((to - from) * i) / segments;
    pts.push(onPlane(center, plane, r * Math.cos(t), r * Math.sin(t)));
  }
  return polyline(pts);
}

/** Full circle in `plane`, as a closed path. */
export function circle(center: V3, r: number, plane: Plane, segments = 32): string {
  const pts: V3[] = [];
  for (let i = 0; i < segments; i++) {
    const t = (Math.PI * 2 * i) / segments;
    pts.push(onPlane(center, plane, r * Math.cos(t), r * Math.sin(t)));
  }
  return poly(pts);
}

/** Axis-aligned box: origin corner (x, y, z) and extents (sx, sy, sz). */
export interface Box {
  x: number;
  y: number;
  z: number;
  sx: number;
  sy: number;
  sz: number;
}

export function box(x: number, y: number, z: number, sx = 1, sy = sx, sz = sx): Box {
  return { x, y, z, sx, sy, sz };
}

/** The three visible faces, drawn back-to-front safe (they never overlap). */
export function faces(b: Box): { top: string; left: string; right: string } {
  const { x, y, z, sx, sy, sz } = b;
  const X = x + sx;
  const Y = y + sy;
  const Z = z + sz;
  return {
    top: poly([
      [x, y, Z],
      [X, y, Z],
      [X, Y, Z],
      [x, Y, Z],
    ]),
    left: poly([
      [x, Y, z],
      [X, Y, z],
      [X, Y, Z],
      [x, Y, Z],
    ]),
    right: poly([
      [X, y, z],
      [X, Y, z],
      [X, Y, Z],
      [X, y, Z],
    ]),
  };
}

/**
 * The mark's three "hidden" spokes, one diagonal brace per visible face:
 * front-top corner to the far corner of the top, left and right faces. With
 * the three real edges that meet at the same corner they make the mark's
 * six-spoke interior.
 */
export function braces(b: Box): string {
  const { x, y, z, sx, sy, sz } = b;
  const c: V3 = [x + sx, y + sy, z + sz];
  return (
    line(c, [x, y, z + sz]) + line(c, [x, y + sy, z]) + line(c, [x + sx, y, z])
  );
}

/** Ground-plane footprint under a box, offset toward the viewer-left as a cast shadow. */
export function shadow(b: Box, spread = 0.18): string {
  const { x, y, sx, sy } = b;
  return poly([
    [x + spread, y + spread, 0],
    [x + sx + spread, y + spread, 0],
    [x + sx + spread, y + sy + spread, 0],
    [x + spread, y + sy + spread, 0],
  ]);
}

/** A rectangle drawn on one of a box's visible faces, inset in face units (0–1). */
export function faceRect(
  b: Box,
  face: "top" | "left" | "right",
  u0: number,
  v0: number,
  u1: number,
  v1: number,
  lift = 0.01,
): string {
  const { x, y, z, sx, sy, sz } = b;
  switch (face) {
    case "top": {
      const Z = z + sz + lift;
      return poly([
        [x + sx * u0, y + sy * v0, Z],
        [x + sx * u1, y + sy * v0, Z],
        [x + sx * u1, y + sy * v1, Z],
        [x + sx * u0, y + sy * v1, Z],
      ]);
    }
    case "left": {
      const Y = y + sy + lift;
      return poly([
        [x + sx * u0, Y, z + sz * v0],
        [x + sx * u1, Y, z + sz * v0],
        [x + sx * u1, Y, z + sz * v1],
        [x + sx * u0, Y, z + sz * v1],
      ]);
    }
    case "right": {
      const X = x + sx + lift;
      return poly([
        [X, y + sy * u0, z + sz * v0],
        [X, y + sy * u1, z + sz * v0],
        [X, y + sy * u1, z + sz * v1],
        [X, y + sy * u0, z + sz * v1],
      ]);
    }
  }
}

/**
 * viewBox covering the projected corners of `boxes` (plus any loose world
 * points), padded in screen pixels.
 */
export function viewBox(boxes: readonly Box[], pad = 12, extra: readonly V3[] = []): string {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  const take = ([px, py]: Pt) => {
    if (px < minX) minX = px;
    if (py < minY) minY = py;
    if (px > maxX) maxX = px;
    if (py > maxY) maxY = py;
  };
  for (const b of boxes) {
    for (const dx of [0, b.sx]) {
      for (const dy of [0, b.sy]) {
        for (const dz of [0, b.sz]) take(project([b.x + dx, b.y + dy, b.z + dz]));
      }
    }
  }
  for (const p of extra) take(project(p));
  return `${f(minX - pad)} ${f(minY - pad)} ${f(maxX - minX + pad * 2)} ${f(maxY - minY + pad * 2)}`;
}
