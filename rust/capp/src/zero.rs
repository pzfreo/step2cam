//! Pickup selection from retained earlier cuts, in the original planning frames.
//! Called after execution exclusions: an attempted cut is never finishing evidence.
use serde_json::{Value, json};
use crate::trace::Trace;

type P = [f64; 3];
fn point(v: &Value) -> Option<P> { Some([v[0].as_f64()?, v[1].as_f64()?, v[2].as_f64()?]) }
fn dot(a: P, b: P) -> f64 { (0..3).map(|i| a[i]*b[i]).sum() }
fn rows(s: &Value) -> [P; 3] { std::array::from_fn(|i| point(&s["rotation"][i]).unwrap()) }
fn local(s: &Value, p: P) -> P {
    let o=point(&s["origin_model"]).unwrap();
    rows(s).map(|r| dot(r, std::array::from_fn(|i| p[i]-o[i])))
}
fn datum(analysis: &Value, id: &Value) -> Option<String> {
    analysis["characteristics"].as_array()?.iter().find(|c| c["kind"]=="datum" && c["faces"].as_array().is_some_and(|fs| fs.contains(id)))?["type"].as_str().map(String::from)
}
fn vertices(mesh: &Value, id: usize) -> Vec<P> {
    let range=&mesh["faces"][id];
    let (Some(start),Some(count))=(range[0].as_u64(),range[1].as_u64()) else { return vec![] };
    (3*start..3*(start+count)).filter_map(|i| {
        let j=mesh["indices"][i as usize].as_u64()? as usize*3;
        Some([mesh["positions"][j].as_f64()?,mesh["positions"][j+1].as_f64()?,mesh["positions"][j+2].as_f64()?])
    }).collect()
}
fn on_edge(p: P, outline: &[Value]) -> bool {
    outline.iter().zip(outline.iter().cycle().skip(1)).take(outline.len()).any(|(a,b)| {
        if a[2].as_f64().unwrap_or(1.).abs()>1e-8 { return false; }
        let (Some(x),Some(y),Some(u),Some(v))=(a[0].as_f64(),a[1].as_f64(),b[0].as_f64(),b[1].as_f64()) else { return false };
        let length=(u-x).powi(2)+(v-y).powi(2);
        let t=if length==0. {0.} else {((p[0]-x)*(u-x)+(p[1]-y)*(v-y))/length}.clamp(0.,1.);
        (p[0]-x-t*(u-x)).hypot(p[1]-y-t*(v-y))<0.02
    })
}
fn finished<'a>(f: &Value, vs: &[P], previous: &'a [Value]) -> Option<&'a Value> {
    if vs.is_empty() { return None; }
    previous.iter().find(|s| {
        let n=rows(s).map(|r|dot(r,point(&f["direction"]).unwrap()));
        let ps: Vec<_>=vs.iter().map(|p|local(s,*p)).collect();
        s["operations"].as_array().into_iter().flatten().filter(|o|o["tool"].is_string()).any(|o| {
            if n[2]>0.999 {
                let levels:Vec<_>=if o["kind"]=="facing" { o["floor_z"].as_f64().into_iter().collect() }
                    else if o["kind"]=="flats" {o["levels"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect()} else {vec![]};
                levels.iter().any(|z|ps.iter().all(|p|(p[2]-z).abs()<0.02))
            } else if n[2].abs()<1e-6 && o["kind"]=="wall_finish" {
                let Some(outline)=o["outer"].as_array() else {return false};
                let floor=o["floor_z"].as_f64().unwrap_or(f64::INFINITY);
                // Only claim the portion above the wall's finishing floor; a full face
                // extending into the retained web is not a finished reference face.
                ps.iter().all(|p|p[2]>=floor-0.02 && on_edge(*p,outline))
            } else { false }
        })
    })
}
fn side_name(axis: usize, sign: f64) -> String {
    format!("{}{}",if sign<0. {"−"} else {"+"},["X","Y","Z"][axis])
}
fn plane_name(a: &Value, s: &Value, f: &Value, finished: bool) -> String {
    datum(a,&f["id"]).map_or_else(|| {
        let n=rows(s).map(|r|dot(r,point(&f["direction"]).unwrap()));
        let axis=(0..3).max_by(|i,j|n[*i].abs().total_cmp(&n[*j].abs())).unwrap();
        format!("the {}{} face",if finished {"finished "} else {""},side_name(axis,n[axis]))
    }, |d|format!("datum {d} face"))
}
fn pickup(description: String, features: Value, setup: Value, method: &str) -> Value {
    json!({"description":description,"features":features,"finished_in":setup,"method":method})
}
fn triangles(stock: &Value) -> Vec<[P;3]> {
    if !stock["mesh"].is_object() && let (Some(lo),Some(hi))=(point(&stock["min_mm"]),point(&stock["max_mm"])) {
        let p:[P;8]=std::array::from_fn(|bits|std::array::from_fn(|i|if bits & (1<<i)==0 {lo[i]} else {hi[i]}));
        return [[0,1,3],[0,3,2],[4,6,7],[4,7,5],[0,4,5],[0,5,1],[2,3,7],[2,7,6],[0,2,6],[0,6,4],[1,5,7],[1,7,3]].map(|t|t.map(|i|p[i])).to_vec();
    }
    stock["mesh"]["indices"].as_array().into_iter().flatten().filter_map(|t| {
        Some([point(&stock["mesh"]["vertices"][t[0].as_u64()? as usize])?,point(&stock["mesh"]["vertices"][t[1].as_u64()? as usize])?,point(&stock["mesh"]["vertices"][t[2].as_u64()? as usize])?])
    }).collect()
}
fn fixtures(s: &Value) -> Vec<(P,P)> {
    ["jaws","parallels"].into_iter().flat_map(|key|s["clamping"][key].as_array().into_iter().flatten())
        .filter_map(|b|Some((point(&b["min"])?,point(&b["max"])?))).collect()
}
fn above(ts: &[[P;3]], q: P, radius: f64) -> bool {
    ts.iter().any(|t|t.iter().any(|v|v[2]>q[2]+0.02) && crate::plan::disk_meets_for_test((q[0],q[1]),radius,t))
}
fn box_above(bs: &[(P,P)], q: P, radius: f64) -> bool {
    bs.iter().any(|(lo,hi)|hi[2]>q[2]+0.02 && (0..2).all(|i|q[i]+radius>lo[i] && q[i]-radius<hi[i]))
}
fn cross(a: P,b: P) -> P { [a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]] }
fn sub(a: P,b: P) -> P { std::array::from_fn(|i|a[i]-b[i]) }
fn segment_hits(a: P,b: P,t: &[P;3]) -> bool {
    let d=sub(b,a);let u=sub(t[1],t[0]);let v=sub(t[2],t[0]);let h=cross(d,v);let det=dot(u,h);
    if det.abs()<1e-10 {return false;}
    let w=sub(a,t[0]);let x=dot(w,h)/det;
    if !(-1e-8..=1.+1e-8).contains(&x) {return false;}
    let q=cross(w,u);let y=dot(d,q)/det;
    let along=dot(v,q)/det;
    y>=-1e-8 && x+y<=1.+1e-8 && (0.0001..0.9999).contains(&along)
}
fn box_hits(a: P,b: P,bs: &[(P,P)]) -> bool {
    bs.iter().any(|(lo,hi)| {
        let (mut enter,mut exit)=(0_f64,1_f64);
        for i in 0..3 {
            let d=b[i]-a[i];
            if d.abs()<1e-10 {if a[i]<lo[i] || a[i]>hi[i] {return false;}}
            else {let x=(lo[i]-a[i])/d;let y=(hi[i]-a[i])/d;enter=enter.max(x.min(y));exit=exit.min(x.max(y));}
        }
        enter<=exit
    })
}
struct Face<'a> {
    f: &'a Value,
    prior: &'a Value,
    p: P,
    n: P,
    vs: Vec<P>,
    probe: Option<P>,
}
// Use triangle centroids and clipped triangle edges, never a centroid outside a
// concave face. The band is above the jaws and below the face's upper edge.
fn band_points(vs: &[P], jaw: f64) -> Vec<P> {
    let mut out=vec![];
    for t in vs.as_chunks::<3>().0 {
        let lo=t.iter().map(|p|p[2]).fold(f64::INFINITY,f64::min).max(jaw+1.);
        let hi=t.iter().map(|p|p[2]).fold(f64::NEG_INFINITY,f64::max);
        if hi-lo<0.1 {continue;}
        let z=(lo+hi)/2.;
        let mut cut=vec![];
        for i in 0..3 {
            let (a,b)=(t[i],t[(i+1)%3]);
            if (a[2]-z)*(b[2]-z)<=0. && (a[2]-b[2]).abs()>1e-8 {
                let k=(z-a[2])/(b[2]-a[2]);cut.push(std::array::from_fn::<_,3,_>(|j|a[j]+k*(b[j]-a[j])));
            }
        }
        if cut.len()>=2 {out.push(std::array::from_fn(|j|(cut[0][j]+cut[1][j])/2.));}
    }
    out
}
fn probe_side(face: &Face<'_>, stock: &[[P;3]], part: &[[P;3]], bs: &[(P,P)], jaw: f64) -> Option<P> {
    let axis=(0..2).find(|i|face.n[*i].abs()>0.999)?;
    band_points(&face.vs,jaw).into_iter().find(|p| {
        let mut q=*p;q[axis]+=face.n[axis];
        if above(stock,q,0.5) || above(part,q,0.5) || box_above(bs,q,0.5) {return false;}
        // The vertical approach must reach the band; the final horizontal
        // approach is swept at +/-0.5 mm in height and along the face.
        for dz in [-0.5,0.,0.5] {for da in [-0.5,0.,0.5] {
            let mut start=q;start[2]+=dz;start[1-axis]+=da;
            let mut end=start;end[axis]=p[axis]+0.05*face.n[axis];
            if box_hits(start,end,bs) || stock.iter().chain(part).any(|t|segment_hits(start,end,t)) {return false;}
        }}
        true
    })
}
fn external(s: &Value, part: &Value, face: &Face<'_>, axis: usize) -> bool {
    let edge=(0..8).map(|bits|local(s,std::array::from_fn(|i|part[if bits & (1<<i)==0 {"min"} else {"max"}][i].as_f64().unwrap()))[axis])
        .reduce(|a,b|if face.n[axis]>0. {a.max(b)} else {a.min(b)}).unwrap();
    (face.p[axis]-edge).abs()<0.02
}
fn fixed_contact(s: &Value, face: &Face<'_>, axis: usize, sign: f64) -> Option<P> {
    if face.n[axis]*sign<0.999 {return None;}
    let c=&s["clamping"];let index=usize::from(sign>0.);
    let jaw=&c["jaws"][index];
    let inner=jaw[if sign<0. {"max"} else {"min"}][axis].as_f64();
    if !inner.is_some_and(|v|(v-face.p[axis]).abs()<0.02) {return None;}
    // A flip has measured, prepared part contact patches. Match the selected
    // planar face to those real patches instead of using the stock envelope.
    s["flip_seating"]["contact_patches_model"].as_array().into_iter().flatten().flat_map(|side|side.as_array().into_iter().flatten()).find_map(|patch| {
        let ps:Vec<_>=patch.as_array().into_iter().flatten().filter_map(point).map(|p|local(s,p)).collect();
        if ps.len()<3 || !ps.iter().all(|p|(p[axis]-face.p[axis]).abs()<0.02) {return None;}
        let q=std::array::from_fn(|i|ps.iter().map(|p|p[i]).sum::<f64>()/ps.len() as f64);
        let project=|p:P|[p[1-axis],p[2],0.];
        (q[2]<=c["jaw_top_z"].as_f64().unwrap_or(f64::NEG_INFINITY)+0.02
            && face.vs.as_chunks::<3>().0.iter().any(|t|crate::plan::disk_meets_for_test((q[1-axis],q[2]),0.01,&t.map(project)))).then_some(q)
    })
}
fn stop_contact(s: &Value, part: &Value, face: &Face<'_>, axis: usize, stock: &[[P;3]], bs: &[(P,P)], jaw: f64) -> Option<P> {
    if !external(s,part,face,axis) {return None;}
    // A stop is set before loading, so it needs a clear horizontal contact band,
    // not a probe's vertical path through the overhanging bar. Keep its nose at
    // least 1 mm outside the nominal wall while testing approach clearance.
    band_points(&face.vs,jaw).into_iter().find(|p| {
        let mut a=*p;let mut b=*p;a[axis]+=4.*face.n[axis];b[axis]+=face.n[axis];
        !box_hits(a,b,bs) && !stock.iter().any(|t|segment_hits(a,b,t))
    })
}
fn axis_pickup(a: &Value, s: &Value, face: &Face<'_>, axis: usize, kind: &str, q: P) -> Value {
    let prior=face.prior["id"].as_str().unwrap();
    let name=plane_name(a,s,face.f,true);
    let description=match kind {
        "fixed_jaw_contact"=>format!("{name} against the fixed jaw (finished in setup {prior}); pick up the fixed jaw before loading the part"),
        "part_stop_contact"=>format!("{name} against a stop at {}0 (finished in setup {prior}); set the stop before loading the part",["X","Y"][axis]),
        _=>format!("{name} (finished in setup {prior}); edge find above the jaws"),
    };
    let mut p=pickup(description,json!([format!("faces:{}",face.f["id"])]),face.prior["id"].clone(),kind);
    p["coordinate_mm"]=json!(0.);
    p["contact_in_previous_frame_mm"]=json!(q);
    p
}
fn stock_touch(s: &Value, analysis: &Value) -> Value {
    let target=analysis["faces"].as_array().into_iter().flatten().find(|f| f["kind"]=="plane"
        && point(&f["direction"]).is_some_and(|n|dot(rows(s)[2],n)>0.999)
        && point(&f["centroid"]).is_some_and(|p|local(s,p)[2].abs()<0.02));
    let name=target.map_or_else(||"the recorded thickness plane".into(),|f|plane_name(analysis,s,f,false));
    let ids=target.map_or_else(||json!([]),|f|json!([format!("faces:{}",f["id"])]));
    let top=s["stock"]["max"][2].as_f64().unwrap();
    pickup(format!("{name}; touch off stock top at Z{}{}; measure the part after facing; bar thickness varies",if top>=0. {"+"} else {""},crate::plan::display_position(top)),ids,Value::Null,"stock_touch_off")
}
fn parallel_zero(s: &Value, a: &Value, faces: &[Face<'_>]) -> Option<Value> {
    let bars=s["clamping"]["parallels"].as_array().filter(|ps|!ps.is_empty())?;
    let z=bars[0]["max"][2].as_f64()?;
    if !bars.iter().all(|p|p["max"][2].as_f64().is_some_and(|v|(v-z).abs()<0.02)) {return None;}
    let bs=fixtures(s);
    let candidates:Vec<_>=faces.iter().filter(|f|f.n[2]< -0.999 && (f.p[2]-z).abs()<0.02).collect();
    let mut seats=vec![];
    for bar in bars {
        let mut support=None;
        for fx in [0.25,0.5,0.75] {for fy in [0.25,0.5,0.75] {
            let q=[bar["min"][0].as_f64()?+fx*(bar["max"][0].as_f64()?-bar["min"][0].as_f64()?),bar["min"][1].as_f64()?+fy*(bar["max"][1].as_f64()?-bar["min"][1].as_f64()?),z];
            if !box_above(&bs,q,0.5) && let Some(f)=candidates.iter().find(|f|f.vs.as_chunks::<3>().0.iter().any(|t|crate::plan::disk_meets_for_test((q[0],q[1]),0.05,t))) {support=Some(*f);}
        }}
        let f=support?;
        if !seats.iter().any(|s: &&Face<'_>|s.f["id"]==f.f["id"]) {seats.push(f);}
    }
    seats.sort_by_key(|f|f.f["id"].as_u64());
    let seat=seats[0];
    let target=a["faces"].as_array().into_iter().flatten().find(|f|f["kind"]=="plane" && point(&f["direction"]).is_some_and(|n|dot(rows(s)[2],n)>0.999) && point(&f["centroid"]).is_some_and(|p|local(s,p)[2].abs()<0.02));
    let name=target.map_or_else(||"the recorded thickness plane".into(),|f|plane_name(a,s,f,false));
    let mut zero=pickup(format!("{name} — {} mm above the parallel tops (seat of {}, finished in setup {}); touch off a parallel before loading the part",crate::plan::display_position(-z),plane_name(a,s,seat.f,false),seat.prior["id"].as_str().unwrap()),json!([format!("faces:{}",seat.f["id"])]),seat.prior["id"].clone(),"parallel_seat");
    let finishes:Vec<_>=seats.iter().map(|f|f.prior["id"].clone()).collect();
    zero["finished_in"]=if finishes.iter().all(|f|*f==finishes[0]) {finishes[0].clone()} else {json!(finishes)};
    zero["features"]=json!(seats.iter().map(|f|format!("faces:{}",f.f["id"])).collect::<Vec<_>>());
    zero["seats"]=json!(seats.iter().map(|f|json!({"feature":format!("faces:{}",f.f["id"]),"finished_in":f.prior["id"]})).collect::<Vec<_>>());
    zero["reference_z_mm"]=json!(z);
    zero["offset_mm"]=json!(-z);
    zero["target_features"]=target.map_or_else(||json!([]),|f|json!([format!("faces:{}",f["id"])]));
    Some(zero)
}

/// Select references after refusals; offsets are old-frame locations of the new
/// origin. Callers translate geometry. Contacts are set before loading the part.
pub fn locate(plan: &mut Value, analysis: &Value, mesh: &Value, stocks: &[Value]) -> Vec<P> {
    let original=plan["setups"].as_array().unwrap().clone();
    let mut trace=Trace::take(plan);let mut shifts=vec![];
    for (index,s) in original.iter().enumerate() {
        let id=s["id"].as_str().unwrap();let jaw=s["clamping"]["fixed_jaw"].as_str().unwrap_or("x.min");
        let fixed_axis=usize::from(jaw.starts_with('y'));let sign=if jaw.ends_with("max") {1.} else {-1.};
        let mut shift=[0.;3];let mut stop=None;let mut rejected=vec![];
        let (xy,z,status)=if index==0 {
            shift=point(&s["stock"]["min"]).unwrap();shift[2]=s["stock"]["max"][2].as_f64().unwrap();
            if sign>0. {shift[fixed_axis]=s["stock"]["max"][fixed_axis].as_f64().unwrap();}
            (pickup(format!("the stock corner at the fixed jaw ({}), −{} end",side_name(fixed_axis,sign),["Y","X"][fixed_axis]),json!([]),Value::Null,"stock_edges"),pickup("stock top".into(),json!([]),Value::Null,"touch off stock top"),"located")
        } else {
            let r=rows(s);let previous=&original[..index];let stock=triangles(stocks.get(index).unwrap_or(&Value::Null));
            let part:Vec<_>=crate::plan::mesh_triangles(mesh).unwrap_or_default().into_iter().map(|t|t.map(|v|local(s,v))).collect();
            let bs=fixtures(s);let jaw_top=s["clamping"]["jaw_top_z"].as_f64().unwrap_or(f64::NEG_INFINITY);
            let mut holes=vec![];let mut capped=std::collections::BTreeSet::new();
            for f in analysis["features"].as_array().into_iter().flatten().filter(|f|f["family"]=="holes") {
                let rec=&f["record"];
                let (Some(axis),Some(location),Some(d))=(point(&rec["axis"]),point(&rec["location"]),rec["diameter"].as_f64()) else {continue};
                if d<6. || dot(r[2],axis).abs()<0.999 || (rec["bottom"]!="through" && dot(r[2],axis)>0.) {continue;}
                for prior in previous {for op in prior["operations"].as_array().into_iter().flatten().filter(|o|o["kind"]=="holes" && o["tool"].is_string() && o["features"].as_array().is_some_and(|fs|fs.contains(&f["id"]))) {
                    if (op["diameter"].as_f64().unwrap_or(0.)-d).abs()>0.01 {continue;}
                    let mut p=local(s,location);
                    p[2]=f["faces"].as_array().into_iter().flatten().filter_map(Value::as_u64).flat_map(|i|vertices(mesh,i as usize)).map(|v|local(s,v)[2]).fold(f64::NEG_INFINITY,f64::max);
                    if p[2]<=jaw_top+1. {continue;}
                    if above(&stock,p,d/2.-0.05) {capped.insert(f["id"].as_str().unwrap().to_owned());continue;}
                    if above(&part,p,d/2.-0.05) || box_above(&bs,p,d/2.-0.05) {continue;}
                    let is_datum=f["faces"].as_array().into_iter().flatten().any(|i|datum(analysis,i).is_some());
                    holes.push((is_datum,d,p,f,prior));
                }}
            }
            if !capped.is_empty() {trace.warn("setup.datum_pickup",id,format!("setup {id}: carried stock covers finished bore pickup mouths; use verified fixture contacts or re-zero on stock"),json!({"features":capped}));}
            holes.sort_by(|a,b|b.0.cmp(&a.0).then_with(||b.1.total_cmp(&a.1)).then_with(||a.3["id"].as_str().cmp(&b.3["id"].as_str())));
            let mut faces=vec![];
            for f in analysis["faces"].as_array().into_iter().flatten().filter(|f|f["kind"]=="plane") {
                let vs=vertices(mesh,f["id"].as_u64().unwrap() as usize);
                let Some(prior)=finished(f,&vs,previous) else {continue};
                let mut face=Face {f,prior,p:local(s,point(&f["centroid"]).unwrap()),n:r.map(|axis|dot(axis,point(&f["direction"]).unwrap())),vs:vs.iter().map(|v|local(s,*v)).collect(),probe:None};
                if face.n[2].abs()<1e-6 {
                    face.probe=probe_side(&face,&stock,&part,&bs,jaw_top);
                    if face.probe.is_none() {rejected.push(json!({"feature":format!("faces:{}",f["id"]),"reason":"probe band or approach obstructed by stock or fixtures"}));}
                }
                faces.push(face);
            }
            faces.sort_by(|a,b|datum(analysis,&a.f["id"]).is_none().cmp(&datum(analysis,&b.f["id"]).is_none()).then_with(||datum(analysis,&a.f["id"]).cmp(&datum(analysis,&b.f["id"]))).then_with(||a.f["id"].as_u64().cmp(&b.f["id"].as_u64())));
            let choose=|axis:usize|faces.iter().filter(|f|f.n[axis].abs()>0.999).find_map(|f| {
                if let Some(q)=if axis==fixed_axis {fixed_contact(s,f,axis,sign)} else {None} {Some((f,"fixed_jaw_contact",q))}
                else if let Some(q)=f.probe {Some((f,"probe_face",q))}
                else if axis!=fixed_axis {stop_contact(s,&plan["part"],f,axis,&stock,&bs,jaw_top).map(|q|(f,"part_stop_contact",q))}
                else {None}
            });
            let pair=choose(0).zip(choose(1));
            let datum_pair=pair.as_ref().is_some_and(|(x,y)|datum(analysis,&x.0.f["id"]).is_some() || datum(analysis,&y.0.f["id"]).is_some());
            // Datum bores and datum face/contact pairs precede non-datum bores.
            let bore=holes.first().filter(|h|h.0 || !datum_pair);
            let xy=if let Some((_,d,p,f,prior))=bore {
                shift[0]=p[0];shift[1]=p[1];
                Some(pickup(format!("centre of ⌀{} bore (cut in setup {})",crate::plan::display_position(*d),prior["id"].as_str().unwrap()),json!([f["id"]]),prior["id"].clone(),"probe_bore"))
            } else if let Some((x,y))=pair {
                shift[0]=x.0.p[0];shift[1]=y.0.p[1];
                let xp=axis_pickup(analysis,s,x.0,0,x.1,x.2);let yp=axis_pickup(analysis,s,y.0,1,y.1,y.2);
                for (axis,choice) in [(0,x),(1,y)] {if choice.1=="part_stop_contact" {
                    stop=Some(json!({"axis":(["x","y"][axis]),"coordinate_mm":0.,"feature":format!("faces:{}",choice.0.f["id"]),"finished_in":choice.0.prior["id"],"contact_z_mm":choice.2[2],"required":true}));
                }}
                let mut p=pickup(format!("{} · {}",xp["description"].as_str().unwrap(),yp["description"].as_str().unwrap()),json!([format!("faces:{}",x.0.f["id"]),format!("faces:{}",y.0.f["id"])]),json!([x.0.prior["id"],y.0.prior["id"]]),if x.1=="probe_face" && y.1=="probe_face" {"probe_faces"} else if x.1=="probe_face" || y.1=="probe_face" {"mixed_references"} else {"fixture_contacts"});
                p["axes"]=json!({"x":xp,"y":yp});Some(p)
            } else {None};
            let mut z=parallel_zero(s,analysis,&faces);
            if z.is_none() && !s["flip_seating"].is_object()
                && let Some(f)=faces.iter().find(|f|f.n[2]>0.999 && f.p[2]>jaw_top+1. && !above(&stock,f.p,0.5) && !above(&part,f.p,0.5) && !box_above(&bs,f.p,0.5)) {
                shift[2]=f.p[2];z=Some(pickup(format!("{} (finished in setup {})",plane_name(analysis,s,f.f,true),f.prior["id"].as_str().unwrap()),json!([format!("faces:{}",f.f["id"])]),f.prior["id"].clone(),"probe_face"));
            }
            if xy.is_none() {
                shift=[0.;3];stop=None;
                if z.as_ref().is_some_and(|z|z["method"]=="probe_face") {z=None;}
                trace.warn("setup.datum_pickup",id,format!("setup {id}: no usable finished feature to locate X/Y; re-zero on the stock edge; frame retained"),json!({}));
            }
            let z=z.unwrap_or_else(|| {
                trace.warn("setup.datum_pickup",id,format!("setup {id}: no finished seat on modelled parallels or reachable finished Z face; measure the part after facing; bar thickness varies"),json!({}));
                stock_touch(s,analysis)
            });
            let status=if xy.is_some() {"located"} else {"warning"};
            (xy.unwrap_or_else(||pickup("recorded X/Y; re-zero on the stock edge".into(),json!([]),Value::Null,"manual_rezero")),z,status)
        };
        let mut zero=json!({"status":status,"xy":xy,"z":z,"offset_in_previous_frame_mm":shift});
        let decision=trace.decide("setup.datum_pickup",id,"chosen","locate from reachable finished references or prepared fixed-jaw, stop and parallel contacts before loading; translate the frame only",json!({"chosen":zero,"rejected_pickups":rejected}));
        zero["because"]=json!([decision]);plan["setups"][index]["zero"]=zero;
        if let Some(stop)=stop {plan["setups"][index]["clamping"]["part_stop"]=stop;}
        shifts.push(shift);
    }
    trace.store(plan);shifts
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capped_faces_and_fixture_obstructions_are_not_probe_pickups() {
        let f=json!({"id":0});let prior=json!({"id":"top"});
        let vs=vec![[0.,0.,-4.],[0.,10.,-4.],[0.,10.,0.],[0.,0.,-4.],[0.,10.,0.],[0.,0.,0.]];
        let face=Face {f:&f,prior:&prior,p:[0.,5.,-2.],n:[-1.,0.,0.],vs,probe:None};
        assert!(probe_side(&face,&[],&[],&[],-6.).is_some());
        let cap=triangles(&json!({"min_mm":[-2.,-2.,0.],"max_mm":[12.,12.,5.]}));
        assert!(probe_side(&face,&cap,&[],&[],-6.).is_none(),"the touch band is below the cap, but the vertical approach is capped");
        let jaw=([ -2.,-1.,-6.],[0.,11.,-1.]);
        assert!(probe_side(&face,&[],&[],&[jaw],-6.).is_none());
        let parallel=([-2.,-1.,-6.],[-0.1,11.,3.]);
        assert!(probe_side(&face,&[],&[],&[parallel],-6.).is_none());
    }
}
