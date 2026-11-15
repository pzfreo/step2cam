export function denseMesh(n) {
  const positions=[],indices=[],faces=[];
  for (let k=0;k<2;k++) for(let y=0;y<=n;y++) for(let x=0;x<=n;x++) positions.push(x*100/n,y*60/n,k?10+2*Math.sin(x/7)*Math.cos(y/5):0);
  const offset=(n+1)**2;
  for(let k=0;k<2;k++) { const start=indices.length/3;
    for(let y=0;y<n;y++) for(let x=0;x<n;x++) { const a=k*offset+y*(n+1)+x,b=a+1,c=b+n+1,d=c-1; indices.push(a,b,c,a,c,d); }
    faces.push([start,indices.length/3-start]);
  }
  return {positions,indices,faces,edges:[]};
}

// Closed round part with deliberately thin, fan-triangulated end faces.
export function cylinderMesh(n = 1000) {
  const positions = [], indices = [], faces = [], edges = [];
  for (const z of [0, 30]) for (let i = 0; i < n; i++) positions.push(20 * Math.cos(i * 2 * Math.PI / n), 20 * Math.sin(i * 2 * Math.PI / n), z);
  positions.push(0, 0, 0, 0, 0, 30);
  for (let i = 0; i < n; i++) indices.push(2 * n, (i + 1) % n, i);
  faces.push([0, n]);
  for (let i = 0; i < n; i++) indices.push(2 * n + 1, n + i, n + (i + 1) % n);
  faces.push([n, n]);
  for (let i = 0; i < n; i++) { const j = (i + 1) % n; indices.push(i, j, n + j, i, n + j, n + i); }
  faces.push([2 * n, 2 * n]);
  for (const z of [0, 30]) { const ring = []; for (let i = 0; i <= n; i++) ring.push(20 * Math.cos(i * 2 * Math.PI / n), 20 * Math.sin(i * 2 * Math.PI / n), z); edges.push(ring); }
  return { positions, indices, faces, edges };
}
export const cylinderSetup = {
  id: 'top', rotation: [[1,0,0],[0,1,0],[0,0,1]], origin_model: [-22,-22,31],
  stock: { min: [0,0,-31], max: [44,44,0] }, zero: 'stock', operations: [{ kind:'facing',floor_z:-1 }],
  clamping: { fixed_jaw:'x.min', faces:['stock.x.min','stock.x.max'], jaw_top_z:-26, grip_height_mm:5,
    jaws: [{min:[-16,-8,-51],max:[0,52,-26]},{min:[44,-8,-51],max:[60,52,-26]}],
    parallels: [{min:[0,4,-51],max:[3,40,-31]},{min:[41,4,-51],max:[44,40,-31]}] }
};
