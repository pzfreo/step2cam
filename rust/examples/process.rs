//! step2cam without a browser: analysis JSON, mesh JSON and tool library in, the result on stdout.
fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [analysis, mesh, tools] = args.as_slice() else {
        return Err("usage: process ANALYSIS.json MESH.json TOOLS.json".into());
    };
    let read = |path: &String| -> Result<serde_json::Value, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        serde_json::from_str(&text).map_err(|e| format!("{path}: {e}"))
    };
    let out = step2cam::process(&read(analysis)?, &read(mesh)?, &read(tools)?)?;
    println!("{out}");
    Ok(())
}
