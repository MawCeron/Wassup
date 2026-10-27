use std::{env, fs, path::Path, process::Command};

fn main() -> std::io::Result<()> {
    prost_build::Config::new()
        // Generated oneofs hold whole messages; boxing them is not worth it here.
        .enum_attribute(".", "#[allow(clippy::large_enum_variant)]")
        .compile_protos(&["../proto/ipc.proto"], &["../proto"])?;
    compile_icons()
}

/// Bundles the interface icons into the binary, so they work uninstalled.
/// GTK looks for them under the app's resource path, as hicolor icons.
fn compile_icons() -> std::io::Result<()> {
    let src = Path::new("../data/icons/hicolor/symbolic/actions");
    println!("cargo:rerun-if-changed={}", src.display());
    let mut files = String::new();
    for entry in fs::read_dir(src)? {
        let name = entry?.file_name().into_string().unwrap();
        files += &format!(
            "<file alias=\"scalable/actions/{name}\" preprocess=\"xml-stripblanks\">{name}</file>\n"
        );
    }
    let out = env::var("OUT_DIR").unwrap();
    let xml = Path::new(&out).join("icons.gresource.xml");
    fs::write(
        &xml,
        format!(
            "<gresources><gresource prefix=\"/io/github/MawCeron/Wassup/icons\">\n{files}</gresource></gresources>\n"
        ),
    )?;
    let status = Command::new("glib-compile-resources")
        .arg("--sourcedir")
        .arg(src)
        .arg("--target")
        .arg(Path::new(&out).join("icons.gresource"))
        .arg(&xml)
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other("glib-compile-resources failed"));
    }
    Ok(())
}
