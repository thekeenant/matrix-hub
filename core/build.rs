//! Generates Rust code for the protobuf definitions in `../proto` and the
//! MTA stop-name lookup table from `../assets/stops.txt`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    compile_protos()?;
    generate_stop_names()?;
    Ok(())
}

fn compile_protos() -> Result<(), Box<dyn std::error::Error>> {
    let (protoc_bin, _) = protoc_prebuilt::init("22.0")
        .map_err(|e| format!("Failed to initialize protoc prebuilt: {e}"))?;
    // buffa-build only reads protoc's location from $PROTOC.
    #[allow(unsafe_code, reason = "set_var; no other threads exist yet")]
    // SAFETY: build scripts are single-threaded at this point.
    unsafe {
        std::env::set_var("PROTOC", protoc_bin);
    }

    println!("cargo:rerun-if-changed=../proto");
    buffa_build::Config::new()
        .generate_json(false)
        .files(&find_protos(Path::new("../proto"))?)
        .includes(&["../proto/"])
        .compile()
        .map_err(|e| format!("Failed to compile protobufs: {e:?}"))?;
    Ok(())
}

fn find_protos(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            files.extend(find_protos(&path)?);
        } else if path.extension().is_some_and(|ext| ext == "proto") {
            files.push(path);
        }
    }
    Ok(files)
}

/// Long secondary names and words shortened to fit the 128px display.
const STOP_NAME_REPLACEMENTS: &[(&str, &str)] = &[
    ("Center", "Ctr"),
    ("Avenue", "Av"),
    ("Parkway", "Pkwy"),
    ("Boulevard", "Blvd"),
    ("Street", "St"),
    ("Square", "Sq"),
    ("Heights", "Hts"),
    ("-Parsons/Archer", ""),
    ("-Washington Hts", ""),
    ("-Lehman College", ""),
    ("-Brooklyn College", ""),
    ("-Medgar Evers College", ""),
    ("-City College", ""),
    ("-Columbia University", ""),
    ("-Lincoln Ctr", ""),
    ("-Museum of Natural History", ""),
    ("-Barclays Ctr", ""),
    ("-Stonewall", ""),
    ("-Little Haiti", ""),
];

/// Writes `stops.rs`: `get_stop_name(parent_stop_id) -> Option<&str>`.
fn generate_stop_names() -> Result<(), Box<dyn std::error::Error>> {
    let stops_path = "../assets/stops.txt";
    println!("cargo:rerun-if-changed={stops_path}");
    let contents = std::fs::read_to_string(stops_path)
        .map_err(|e| format!("Failed to read {stops_path}: {e}"))?;

    let mut out = String::from(
        "/// Display name of a parent (non-directional) GTFS stop ID.\n\
         pub fn get_stop_name(stop_id: &str) -> Option<&'static str> {\n    match stop_id {\n",
    );
    // Skip the CSV header: stop_id,stop_name,...
    for line in contents.lines().skip(1) {
        let mut fields = line.split(',');
        let (Some(stop_id), Some(name)) = (fields.next(), fields.next()) else {
            continue;
        };
        // Directional platform IDs (e.g. 701N) share the parent's name
        if stop_id.ends_with('N') || stop_id.ends_with('S') {
            continue;
        }
        let name = STOP_NAME_REPLACEMENTS
            .iter()
            .fold(name.to_string(), |n, (from, to)| n.replace(from, to));
        writeln!(out, "        {stop_id:?} => Some({name:?}),")?;
    }
    out.push_str("        _ => None,\n    }\n}\n");

    let dest = Path::new(&std::env::var("OUT_DIR")?).join("stops.rs");
    std::fs::write(dest, out)?;
    Ok(())
}
