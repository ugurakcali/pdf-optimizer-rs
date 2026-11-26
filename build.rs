use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
        let res_obj = out_dir.join("app_icon.o");
        let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
        let rc_path = manifest_dir.join("assets").join("app.rc");

        let status = Command::new("windres")
            .args([
                rc_path.to_str().expect("valid rc path"),
                "-O",
                "coff",
                "-o",
                res_obj.to_str().expect("valid obj path"),
            ])
            .status()
            .expect("windres failed to start");

        assert!(status.success(), "windres failed with status {status}");
        println!("cargo:rustc-link-arg={}", res_obj.display());
    }
}
