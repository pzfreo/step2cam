// The stock block, cut away as the program plays. Each setup keeps a height field over its own
// stock: every cutting move lowers it under the tool's shape. The block drawn is what lies under
// every setup's height field at once, found by marching each pixel's ray through the stock box in
// a shader, so cuts from the side setups show on the same block as those from the top.
//
// This is for looking at, not a check: camlib's comparison of the stock with the part is the
// check. A height field cannot hold an undercut, and a thread mill is drawn as a flat disc.

import * as THREE from "three";

const STEPS = 480;

// How far above its tip the tool's surface is at distance d from its axis.
function profile(tool) {
  const radius = (tool?.diameter || 3) / 2;
  if (tool?.type === "drill") {
    const slope = 1 / Math.tan((tool.point_angle || 118) / 2 * Math.PI / 180);
    return { radius, at: (d) => d * slope };
  }
  if (tool?.type === "chamfer_mill") {
    const slope = 1 / Math.tan((tool.angle || 90) / 2 * Math.PI / 180), tip = (tool.tip_diameter || 0) / 2;
    return { radius, at: (d) => Math.max(0, d - tip) * slope };
  }
  return { radius, at: () => 0 };
}

export function createStock(model, toModel) {
  // The mill's setups: a lathe setup is planned, not posted, and has no block of stock here.
  const setups = model.data.plan.setups.filter((s) => s.kind !== "lathe");
  const fields = setups.map((setup) => {
    const lo = setup.stock.min, hi = setup.stock.max;
    const cell = Math.max(Math.max(hi[0] - lo[0], hi[1] - lo[1]) / 400, 0.1);
    const nx = Math.max(1, Math.ceil((hi[0] - lo[0]) / cell)), ny = Math.max(1, Math.ceil((hi[1] - lo[1]) / cell));
    const heights = new Float32Array(nx * ny).fill(hi[2]);
    const texture = new THREE.DataTexture(heights, nx, ny, THREE.RedFormat, THREE.FloatType);
    texture.magFilter = texture.minFilter = THREE.NearestFilter;
    texture.needsUpdate = true;
    return { setup, lo, hi, nx, ny, cell: [(hi[0] - lo[0]) / nx, (hi[1] - lo[1]) / ny], heights, texture, top: hi[2] };
  });
  const index = new Map(setups.map((s, i) => [s, i]));
  const shapes = model.segments.map((s) => profile(s.tool));

  const home = setups.find((s) => s.zero === "stock") || setups[0];
  const a = toModel(home, home.stock.min), b = toModel(home, home.stock.max);
  const boxMin = new THREE.Vector3(Math.min(a[0], b[0]), Math.min(a[1], b[1]), Math.min(a[2], b[2]));
  const boxMax = new THREE.Vector3(Math.max(a[0], b[0]), Math.max(a[1], b[1]), Math.max(a[2], b[2]));

  const uniforms = { boxMin: { value: boxMin }, boxMax: { value: boxMax }, rayDir: { value: new THREE.Vector3(0, 0, -1) } };
  fields.forEach((f, k) => {
    const r = f.setup.rotation;
    // Model -> setup: q = R (p - o), with the rotation's rows as given.
    uniforms[`rot${k}`] = { value: new THREE.Matrix3().set(...r[0], ...r[1], ...r[2]) };
    uniforms[`origin${k}`] = { value: new THREE.Vector3(...f.setup.origin_model) };
    uniforms[`area${k}`] = { value: new THREE.Vector4(f.lo[0], f.lo[1], f.hi[0], f.hi[1]) };
    uniforms[`size${k}`] = { value: new THREE.Vector2(f.nx, f.ny) };
    uniforms[`top${k}`] = { value: f.top };
    uniforms[`heights${k}`] = { value: f.texture };
  });

  const each = (body) => fields.map((_, k) => body(k)).join("\n");
  const material = new THREE.ShaderMaterial({
    glslVersion: THREE.GLSL3,
    uniforms,
    vertexShader: `
      out vec3 vWorld;
      void main() {
        vWorld = (modelMatrix * vec4(position, 1.0)).xyz;
        gl_Position = projectionMatrix * viewMatrix * vec4(vWorld, 1.0);
      }`,
    fragmentShader: `
      precision highp float;
      precision highp sampler2D;
      in vec3 vWorld;
      out vec4 fragColor;
      uniform vec3 boxMin, boxMax, rayDir;
      uniform mat4 projectionMatrix;
      ${each((k) => `uniform sampler2D heights${k}; uniform mat3 rot${k}; uniform vec3 origin${k}; uniform vec4 area${k}; uniform vec2 size${k}; uniform float top${k};`)}

      float heightAt(sampler2D field, vec2 uv) { return texture(field, uv).r; }

      bool inside(vec3 p) {
        if (any(lessThan(p, boxMin - 1e-4)) || any(greaterThan(p, boxMax + 1e-4))) return false;
        ${each((k) => `{
          vec3 q = rot${k} * (p - origin${k});
          vec2 uv = (q.xy - area${k}.xy) / (area${k}.zw - area${k}.xy);
          if (all(greaterThanEqual(uv, vec2(0.0))) && all(lessThanEqual(uv, vec2(1.0))) && q.z > heightAt(heights${k}, uv) + 1e-4) return false;
        }`)}
        return true;
      }

      // The surface met: the constraint closest to its limit, and whether it has been cut.
      vec3 surface(vec3 p, out float cut) {
        float best = 1e9;
        vec3 n = vec3(0.0, 0.0, 1.0);
        cut = 0.0;
        vec3 low = p - boxMin, high = boxMax - p;
        if (low.x < best) { best = low.x; n = vec3(-1, 0, 0); }
        if (high.x < best) { best = high.x; n = vec3(1, 0, 0); }
        if (low.y < best) { best = low.y; n = vec3(0, -1, 0); }
        if (high.y < best) { best = high.y; n = vec3(0, 1, 0); }
        if (low.z < best) { best = low.z; n = vec3(0, 0, -1); }
        if (high.z < best) { best = high.z; n = vec3(0, 0, 1); }
        ${each((k) => `{
          vec3 q = rot${k} * (p - origin${k});
          vec2 span = area${k}.zw - area${k}.xy;
          vec2 uv = (q.xy - area${k}.xy) / span;
          if (all(greaterThanEqual(uv, vec2(0.0))) && all(lessThanEqual(uv, vec2(1.0)))) {
            float h = heightAt(heights${k}, uv);
            float margin = h - q.z;
            if (margin < best) {
              best = margin;
              vec2 texel = 1.0 / size${k}, cell = span / size${k};
              float gx = (heightAt(heights${k}, uv + vec2(texel.x, 0.0)) - heightAt(heights${k}, uv - vec2(texel.x, 0.0))) / (2.0 * cell.x);
              float gy = (heightAt(heights${k}, uv + vec2(0.0, texel.y)) - heightAt(heights${k}, uv - vec2(0.0, texel.y))) / (2.0 * cell.y);
              n = transpose(rot${k}) * normalize(vec3(-gx, -gy, 1.0));
              cut = h < top${k} - 1e-3 ? 1.0 : 0.0;
            }
          }
        }`)}
        return normalize(n);
      }

      void main() {
        vec3 dir = normalize(rayDir);
        vec3 o = vWorld - dir * 1e-3;
        vec3 inv = 1.0 / dir;
        vec3 t0 = (boxMin - o) * inv, t1 = (boxMax - o) * inv;
        vec3 far = max(t0, t1);
        float exit = min(min(far.x, far.y), far.z);
        float stepLen = exit / float(${STEPS});
        float hit = -1.0;
        for (int i = 0; i <= ${STEPS}; i++) {
          float t = float(i) * stepLen;
          if (inside(o + dir * t)) {
            float a = max(t - stepLen, 0.0), b = t;
            for (int j = 0; j < 10; j++) {
              float m = 0.5 * (a + b);
              if (inside(o + dir * m)) b = m; else a = m;
            }
            hit = b;
            break;
          }
        }
        if (hit < 0.0) discard;
        vec3 p = o + dir * hit;
        float cut;
        vec3 n = surface(p, cut);
        if (dot(n, dir) > 0.0) n = -n;
        vec3 light = normalize(-dir + vec3(0.35, 0.25, 0.6));
        float lit = 0.35 + 0.65 * max(dot(n, light), 0.0);
        vec3 raw = vec3(0.47, 0.51, 0.56), machined = vec3(0.78, 0.82, 0.86);
        fragColor = vec4(mix(raw, machined, cut) * lit, 1.0);
        vec4 clip = projectionMatrix * viewMatrix * vec4(p, 1.0);
        gl_FragDepth = clamp(clip.z / clip.w * 0.5 + 0.5, 0.0, 1.0);
      }`,
  });
  const size = new THREE.Vector3().subVectors(boxMax, boxMin);
  const mesh = new THREE.Mesh(new THREE.BoxGeometry(size.x, size.y, size.z), material);
  mesh.position.copy(boxMin).addScaledVector(size, 0.5);

  let done = 0;  // segments cut into the height fields so far
  const local = model.local;

  function cutSegment(i) {
    const s = model.segments[i];
    if (s.rapid || !s.tool) return null;
    const k = index.get(s.setup), f = fields[k], shape = shapes[i];
    const at = 6 * i;
    const [ax, ay, az, bx, by, bz] = local.slice(at, at + 6);
    const r = shape.radius;
    const dx = bx - ax, dy = by - ay, length2 = dx * dx + dy * dy;
    const x0 = Math.max(0, Math.floor((Math.min(ax, bx) - r - f.lo[0]) / f.cell[0]));
    const x1 = Math.min(f.nx - 1, Math.floor((Math.max(ax, bx) + r - f.lo[0]) / f.cell[0]));
    const y0 = Math.max(0, Math.floor((Math.min(ay, by) - r - f.lo[1]) / f.cell[1]));
    const y1 = Math.min(f.ny - 1, Math.floor((Math.max(ay, by) + r - f.lo[1]) / f.cell[1]));
    for (let iy = y0; iy <= y1; iy++) {
      const cy = f.lo[1] + (iy + 0.5) * f.cell[1];
      for (let ix = x0; ix <= x1; ix++) {
        const cx = f.lo[0] + (ix + 0.5) * f.cell[0];
        const t = length2 > 0 ? Math.max(0, Math.min(1, ((cx - ax) * dx + (cy - ay) * dy) / length2)) : 0;
        const d = Math.hypot(cx - (ax + dx * t), cy - (ay + dy * t));
        if (d > r) continue;
        // A plunge is as deep as its lower end.
        const tip = length2 > 0 ? az + (bz - az) * t : Math.min(az, bz);
        const z = tip + shape.at(d);
        const cellAt = iy * f.nx + ix;
        if (z < f.heights[cellAt]) f.heights[cellAt] = z;
      }
    }
    return f;
  }

  // Cut everything before segment `upTo`; going back starts again from the whole block.
  function update(upTo) {
    if (upTo < done) {
      for (const f of fields) f.heights.fill(f.top);
      done = 0;
      for (const f of fields) f.texture.needsUpdate = true;
    }
    for (; done < upTo; done++) {
      const f = cutSegment(done);
      if (f) f.texture.needsUpdate = true;
    }
  }

  return {
    mesh,
    update,
    aim(camera) { camera.getWorldDirection(uniforms.rayDir.value); },
    dispose() {
      mesh.geometry.dispose();
      material.dispose();
      for (const f of fields) f.texture.dispose();
    },
  };
}
