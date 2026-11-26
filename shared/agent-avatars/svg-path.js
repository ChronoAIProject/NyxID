/** Portable SVG contour sampler. Used by the generator and deterministic PNG rasterizer; clients sample generated contours. */
export function contours(d) {
  const tokens =
    d.match(/[a-zA-Z]|[-+]?(?:\d*\.\d+|\d+\.?\d*)(?:[eE][-+]?\d+)?/g) ?? [];
  let i = 0,
    command = "",
    x = 0,
    y = 0,
    sx = 0,
    sy = 0;
  const paths = [];
  let path = [];
  const push = (px, py) => {
    x = px;
    y = py;
    path.push([x, y]);
  };
  const num = () => Number(tokens[i++]);
  while (i < tokens.length) {
    if (/^[a-zA-Z]$/.test(tokens[i])) command = tokens[i++];
    const rel = command === command.toLowerCase(),
      kind = command.toUpperCase();
    const ox = x,
      oy = y;
    const point = () => [num() + (rel ? ox : 0), num() + (rel ? oy : 0)];
    if (kind === "Z") {
      push(sx, sy);
      command = "";
      continue;
    }
    if (kind === "M") {
      if (path.length) paths.push(path);
      path = [];
      const [px, py] = point();
      push(px, py);
      sx = x;
      sy = y;
      command = rel ? "l" : "L";
    } else if (kind === "L") {
      const [px, py] = point();
      push(px, py);
    } else if (kind === "H") push(num() + (rel ? ox : 0), y);
    else if (kind === "V") push(x, num() + (rel ? oy : 0));
    else if (kind === "C" || kind === "Q") {
      const a = point(),
        b = point(),
        c = kind === "C" ? point() : b;
      for (let k = 1; k <= 48; k++) {
        const t = k / 48,
          u = 1 - t;
        push(
          kind === "C"
            ? u * u * u * ox +
                3 * u * u * t * a[0] +
                3 * u * t * t * b[0] +
                t * t * t * c[0]
            : u * u * ox + 2 * u * t * a[0] + t * t * b[0],
          kind === "C"
            ? u * u * u * oy +
                3 * u * u * t * a[1] +
                3 * u * t * t * b[1] +
                t * t * t * c[1]
            : u * u * oy + 2 * u * t * a[1] + t * t * b[1],
        );
      }
    } else if (kind === "A") {
      let rx = Math.abs(num()),
        ry = Math.abs(num());
      const phi = (num() * Math.PI) / 180,
        large = num(),
        sweep = num(),
        end = point();
      if (!rx || !ry) {
        push(...end);
        continue;
      }
      const cp = Math.cos(phi),
        sp = Math.sin(phi),
        dx = (ox - end[0]) / 2,
        dy = (oy - end[1]) / 2;
      const xp = cp * dx + sp * dy,
        yp = -sp * dx + cp * dy,
        lambda = (xp * xp) / (rx * rx) + (yp * yp) / (ry * ry);
      if (lambda > 1) {
        rx *= Math.sqrt(lambda);
        ry *= Math.sqrt(lambda);
      }
      const factor =
        (large === sweep ? -1 : 1) *
        Math.sqrt(
          Math.max(
            0,
            (rx * rx * ry * ry - rx * rx * yp * yp - ry * ry * xp * xp) /
              (rx * rx * yp * yp + ry * ry * xp * xp),
          ),
        );
      const cxp = (factor * rx * yp) / ry,
        cyp = (-factor * ry * xp) / rx,
        cx = cp * cxp - sp * cyp + (ox + end[0]) / 2,
        cy = sp * cxp + cp * cyp + (oy + end[1]) / 2;
      const a = Math.atan2((yp - cyp) / ry, (xp - cxp) / rx),
        b = Math.atan2((-yp - cyp) / ry, (-xp - cxp) / rx);
      let delta = b - a;
      if (!sweep && delta > 0) delta -= 2 * Math.PI;
      if (sweep && delta < 0) delta += 2 * Math.PI;
      const steps = Math.max(8, Math.ceil(Math.abs(delta) * 24));
      for (let k = 1; k <= steps; k++) {
        const t = a + (delta * k) / steps;
        push(
          cx + cp * rx * Math.cos(t) - sp * ry * Math.sin(t),
          cy + sp * rx * Math.cos(t) + cp * ry * Math.sin(t),
        );
      }
    } else throw new Error(`Unsupported SVG command ${command}`);
  }
  if (path.length) paths.push(path);
  return paths;
}
