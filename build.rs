//! Sets up the ESP-IDF build environment and builds the web frontend that
//! gets embedded in the firmware. (Protobuf and stop-name code generation
//! happens in `core/build.rs`.)

fn main() {
    embuild::espidf::sysenv::output();

    // Commit this firmware was built from (shown in /api/debug)
    let git = std::process::Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map_or_else(
            || "unknown".into(),
            |o| String::from_utf8_lossy(&o.stdout).trim().to_string(),
        );
    println!("cargo:rustc-env=MATRIX_HUB_GIT={git}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");

    println!("cargo:rerun-if-changed=frontend/src");
    println!("cargo:rerun-if-changed=frontend/index.html");
    println!("cargo:rerun-if-changed=frontend/package.json");
    println!("cargo:rerun-if-changed=frontend/vite.config.ts");
    println!("cargo:rerun-if-changed=proto/");

    match std::process::Command::new("npm")
        .current_dir("frontend")
        .args(["run", "build"])
        .status()
    {
        Ok(status) if !status.success() => {
            panic!("npm run build failed in the frontend directory");
        }
        Err(e) => {
            println!("cargo:warning=Failed to run npm build in frontend directory: {e}");
        }
        Ok(_) => {}
    }
}
