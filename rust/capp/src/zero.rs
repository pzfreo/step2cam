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
fn face_name(a: &Value, f: &Value) -> String {
    datum(a,&f["id"]).map_or_else(|| format!("face {}",f["id"]), |d| format!("datum {d} face"))
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
fn stock_touch(s: &Value, analysis: &Value) -> Value {
    let mut faces:Vec<_>=analysis["faces"].as_array().into_iter().flatten().filter(|f| {
        f["kind"]=="plane" && point(&f["direction"]).is_some_and(|n|dot(rows(s)[2],n)>0.999)
            && point(&f["centroid"]).is_some_and(|p|local(s,p)[2].abs()<0.02)
    }).collect();
    faces.sort_by_key(|f|datum(analysis,&f["id"]).unwrap_or_else(||"~".into()));
    let (name,ids)=faces.first().map_or_else(||("recorded Z plane".into(),json!([])),|f|
        (format!("{} (not yet finished)",face_name(analysis,f)),json!([format!("faces:{}",f["id"])])));
    let top=s["stock"]["max"][2].as_f64().unwrap();
    pickup(format!("{name}; touch off stock top at Z{}{}",if top>=0. {"+"} else {""},crate::plan::show(top)),ids,Value::Null,"touch off stock top using recorded offset")
}
fn pickup(description: String, features: Value, setup: Value, method: &str) -> Value {
    json!({"description":description,"features":features,"finished_in":setup,"method":method})
}

/// Record each zero and return the old-frame location of the new origin. Missing XY
/// evidence keeps the complete frame, with a warning. All geometry is translated by callers.
pub fn locate(plan: &mut Value, analysis: &Value, mesh: &Value, stocks: &[Value]) -> Vec<P> {
    let original=plan["setups"].as_array().unwrap().clone();
    let mut trace=Trace::take(plan);
    let mut shifts=vec![];
    for (index,s) in original.iter().enumerate() {
        let id=s["id"].as_str().unwrap();
        let jaw=s["clamping"]["fixed_jaw"].as_str().unwrap_or("x.min");
        let mut shift=[0.;3];
        let (xy,z,status)=if index==0 {
            shift=point(&s["stock"]["min"]).unwrap();
            shift[2]=s["stock"]["max"][2].as_f64().unwrap();
            if jaw.ends_with("max") {let axis=usize::from(jaw.starts_with('y'));shift[axis]=s["stock"]["max"][axis].as_f64().unwrap();}
            (pickup(format!("stock corner against fixed jaw {jaw}, minimum of the other axis"),json!([]),Value::Null,"edge find the two stock edges"),
             pickup("stock top".into(),json!([]),Value::Null,"touch off stock top"),"located")
        } else {
            let r=rows(s);
            let triangles:Vec<_>=crate::plan::mesh_triangles(mesh).unwrap_or_default().into_iter().map(|t|t.map(|v|local(s,v))).collect();
            let jaw_top=s["clamping"]["jaw_top_z"].as_f64().unwrap_or(f64::NEG_INFINITY);
            let previous=&original[..index];
            let mut holes=vec![];
            for f in analysis["features"].as_array().into_iter().flatten().filter(|f|f["family"]=="holes") {
                let rec=&f["record"];
                let (Some(axis),Some(location),Some(d))=(point(&rec["axis"]),point(&rec["location"]),rec["diameter"].as_f64()) else {continue};
                if d<6. || dot(r[2],axis).abs()<0.999 {continue;}
                for prior in previous {
                    for op in prior["operations"].as_array().into_iter().flatten().filter(|o|o["kind"]=="holes" && o["tool"].is_string() && o["features"].as_array().is_some_and(|fs|fs.contains(&f["id"]))) {
                        if (op["diameter"].as_f64().unwrap_or(0.)-d).abs()>0.01 {continue;}
                        // A reversed pickup requires a through bore. Blind bores must
                        // still face upwards in the receiving setup.
                        if rec["bottom"]!="through" && dot(r[2],axis)>0. {continue;}
                        let p=local(s,location);
                        let ends:Vec<_>=f["faces"].as_array().into_iter().flatten().filter_map(Value::as_u64).flat_map(|i|vertices(mesh,i as usize)).map(|v|local(s,v)).collect();
                        let mouth=ends.iter().map(|p|p[2]).fold(f64::NEG_INFINITY,f64::max);
                        if mouth<=jaw_top+1. {continue;}
                        let stock=&stocks.get(index).unwrap_or(&Value::Null)["mesh"];
                        let capped=stock["indices"].as_array().into_iter().flatten().filter_map(|t| {
                            Some([point(&stock["vertices"][t[0].as_u64()? as usize])?,point(&stock["vertices"][t[1].as_u64()? as usize])?,point(&stock["vertices"][t[2].as_u64()? as usize])?])
                        }).any(|t|t.iter().any(|v|v[2]>mouth+0.02) && crate::plan::disk_meets_for_test((p[0],p[1]),d/2.-0.05,&t));
                        if capped {
                            trace.warn("setup.datum_pickup",id,format!("setup {id}: {} is finished in setup {} but carried stock caps its pickup mouth; use exposed finished faces or re-zero on stock",f["id"].as_str().unwrap(),prior["id"].as_str().unwrap()),json!({"features":[f["id"]]}));
                            continue;
                        }
                        let blocked=triangles.iter().any(|t|t.iter().any(|v|v[2]>mouth+0.02) && crate::plan::disk_meets_for_test((p[0],p[1]),d/2.-0.05,t));
                        if blocked {continue;}
                        let is_datum=f["faces"].as_array().into_iter().flatten().any(|i|datum(analysis,i).is_some());
                        holes.push((is_datum,d,p,f,prior));
                    }
                }
            }
            holes.sort_by(|a,b|b.0.cmp(&a.0).then_with(||b.1.total_cmp(&a.1)).then_with(||a.3["id"].as_str().cmp(&b.3["id"].as_str())));
            let mut faces=vec![];
            for f in analysis["faces"].as_array().into_iter().flatten().filter(|f|f["kind"]=="plane") {
                let vs=vertices(mesh,f["id"].as_u64().unwrap() as usize);
                let Some(prior)=finished(f,&vs,previous) else {continue};
                let p=local(s,point(&f["centroid"]).unwrap());
                let n=r.map(|axis|dot(axis,point(&f["direction"]).unwrap()));
                let high=vs.iter().map(|v|local(s,*v)[2]).fold(f64::NEG_INFINITY,f64::max);
                if high<=jaw_top+1. {continue;}
                // External axis-aligned side faces are reachable from outside; upward
                // faces need a clear vertical approach at a mesh triangle centroid.
                let side_axis=(0..2).find(|i|n[*i].abs()>0.999);
                let external=side_axis.is_some_and(|axis| {
                    let bounds:Vec<_>=(0..8).map(|bits|local(s,std::array::from_fn(|i|plan["part"][if bits & (1<<i)==0 {"min"} else {"max"}][i].as_f64().unwrap()))[axis]).collect();
                    let edge=if n[axis]>0. {bounds.into_iter().fold(f64::NEG_INFINITY,f64::max)} else {bounds.into_iter().fold(f64::INFINITY,f64::min)};
                    (p[axis]-edge).abs()<0.02
                });
                let exposed=external || (n[2]>0.999 && vs.as_chunks::<3>().0.iter().any(|t| {
                    let c=std::array::from_fn(|i|(t[0][i]+t[1][i]+t[2][i])/3.);
                    let q=local(s,c);
                    !triangles.iter().any(|t|t.iter().any(|v|v[2]>q[2]+0.02) && crate::plan::disk_meets_for_test((q[0],q[1]),0.05,t))
                }));
                if exposed {faces.push((datum(analysis,&f["id"]),f,prior,p,n));}
            }
            faces.sort_by(|a,b|b.0.is_some().cmp(&a.0.is_some()).then_with(||a.0.cmp(&b.0)).then_with(||a.1["id"].as_u64().cmp(&b.1["id"].as_u64())));
            let side=|axis:usize|faces.iter().find(|f|f.4[axis].abs()>0.999);
            let xy=if let Some((_,d,p,f,prior))=holes.first() {
                shift[0]=p[0];shift[1]=p[1];
                Some(pickup(format!("centre of ⌀{} bore (cut in setup {})",crate::plan::show(*d),prior["id"].as_str().unwrap()),json!([f["id"]]),prior["id"].clone(),"probe bore centre or sweep with edge finder"))
            } else if let (Some(x),Some(y))=(side(0),side(1)) {
                shift[0]=x.3[0];shift[1]=y.3[1];
                Some(pickup(format!("corner of {} and {}",face_name(analysis,x.1),face_name(analysis,y.1)),json!([format!("faces:{}",x.1["id"]),format!("faces:{}",y.1["id"])]),json!([x.2["id"],y.2["id"]]),"edge find perpendicular finished faces above jaws"))
            } else {None};
            let z=faces.iter().find(|f|f.4[2]>0.999).map(|f| {
                shift[2]=f.3[2];
                pickup(format!("{} (finished in setup {})",face_name(analysis,f.1),f.2["id"].as_str().unwrap()),json!([format!("faces:{}",f.1["id"])]),f.2["id"].clone(),"touch off finished face from above")
            });
            if let Some(xy)=xy {
                let z=z.unwrap_or_else(|| {
                    // No previously finished upward face: retain the existing Z plane,
                    // whose offset from actual reachable stock is recorded, never guessed.
                    stock_touch(s,analysis)
                });
                (xy,z,"located")
            } else {
                shift=[0.;3];
                let warning=format!("setup {id}: no finished feature to pick up X/Y; re-zero on the stock edge; frame retained");
                trace.warn("setup.datum_pickup",id,warning,json!({}));
                (pickup("recorded X/Y; re-zero on the stock edge".into(),json!([]),Value::Null,"manual re-zero; no finished pickup"),
                 stock_touch(s,analysis),"warning")
            }
        };
        let mut zero=json!({"status":status,"xy":xy,"z":z,"offset_in_previous_frame_mm":shift});
        let decision=trace.decide("setup.datum_pickup",id,"chosen","record zero from retained earlier finishing and reachable pickup surfaces; translate the setup frame only",json!({"chosen":zero}));
        zero["because"]=json!([decision]);
        plan["setups"][index]["zero"]=zero;
        shifts.push(shift);
    }
    trace.store(plan);
    shifts
}
