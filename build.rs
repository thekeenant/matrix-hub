fn main() -> Result<(), Box<dyn std::error::Error>> {
    embuild::espidf::sysenv::output();

    let (protoc_bin, _) = protoc_prebuilt::init("22.0")
        .map_err(|e| format!("Failed to initialize protoc prebuilt: {e}"))?;
    #[allow(
        unsafe_code,
        reason = "Required to set PROTOC environment variable for protoc_prebuilt"
    )]
    unsafe {
        std::env::set_var("PROTOC", protoc_bin);
    }

    fn find_protos(dir: &std::path::Path) -> std::io::Result<Vec<std::path::PathBuf>> {
        let mut files = Vec::new();
        if dir.is_dir() {
            for entry in std::fs::read_dir(dir)? {
                let path = entry?.path();
                if path.is_dir() {
                    files.extend(find_protos(&path)?);
                } else if path.extension().is_some_and(|ext| ext == "proto") {
                    files.push(path);
                }
            }
        }
        Ok(files)
    }

    buffa_build::Config::new()
        .generate_json(false) // No need for JSON generation since we fetch protobufs directly
        .files(&find_protos(std::path::Path::new("proto"))?)
        .includes(&["proto/"])
        .compile()
        .map_err(|e| format!("Failed to compile protobufs: {e:?}"))?;

    // Generate stops mapping
    let out_dir = std::env::var("OUT_DIR")?;
    let dest_path = std::path::Path::new(&out_dir).join("stops.rs");
    let mut out = String::new();
    out.push_str("pub fn get_stop_name(stop_id: &str) -> Option<&'static str> {\n");
    out.push_str("    match stop_id {\n");

    let stops_txt_path = "assets/stops.txt";
    if std::path::Path::new(stops_txt_path).exists() {
        println!("cargo:rerun-if-changed={stops_txt_path}");
        let contents = std::fs::read_to_string(stops_txt_path)?;
        let mut is_first = true;

        for line in contents.lines() {
            if is_first {
                is_first = false;
                continue;
            }

            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 3 {
                let stop_id = parts[0];
                let mut stop_name = parts[1].to_string();

                // Common abbreviations and removals of long co-names
                stop_name = stop_name.replace("Center", "Ctr");
                stop_name = stop_name.replace("Avenue", "Av");
                stop_name = stop_name.replace("Parkway", "Pkwy");
                stop_name = stop_name.replace("Boulevard", "Blvd");
                stop_name = stop_name.replace("Street", "St");
                stop_name = stop_name.replace("Square", "Sq");
                stop_name = stop_name.replace("Heights", "Hts");

                // Remove long, unnecessary secondary names that take up matrix space
                stop_name = stop_name.replace("-Parsons/Archer", "");
                stop_name = stop_name.replace("-Washington Hts", "");
                stop_name = stop_name.replace("-Lehman College", "");
                stop_name = stop_name.replace("-Brooklyn College", "");
                stop_name = stop_name.replace("-Medgar Evers College", "");
                stop_name = stop_name.replace("-City College", "");
                stop_name = stop_name.replace("-Columbia University", "");
                stop_name = stop_name.replace("-Lincoln Center", "");
                stop_name = stop_name.replace("-Museum of Natural History", "");
                stop_name = stop_name.replace("-Barclays Ctr", "");
                stop_name = stop_name.replace("-Stonewall", "");
                stop_name = stop_name.replace("-Little Haiti", "");

                // Exclude directional stop IDs (e.g., 701N)
                if !stop_id.ends_with('N') && !stop_id.ends_with('S') {
                    out.push_str(&format!(
                        "        \"{}\" => Some(\"{}\"),\n",
                        stop_id,
                        stop_name.replace('"', "\\\"")
                    ));
                }
            }
        }
    } else {
        println!(
            "cargo:warning=assets/stops.txt not found! Destination names won't be mapped properly."
        );
    }

    out.push_str("        _ => None,\n");
    out.push_str("    }\n");
    out.push_str("}\n");

    std::fs::write(&dest_path, out)?;

    Ok(())
}
