use std::{env, fs, path::Path, process::Command};

fn main() -> std::io::Result<()> {
    prost_build::Config::new()
        // Generated oneofs hold whole messages; boxing them is not worth it here.
        .enum_attribute(".", "#[allow(clippy::large_enum_variant)]")
        .compile_protos(&["../proto/ipc.proto"], &["../proto"])?;
    compile_resources()
}

/// Bundles the interface icons and chat backgrounds into the binary, so
/// they work uninstalled. GTK looks for the icons under the app's resource
/// path, as hicolor icons.
fn compile_resources() -> std::io::Result<()> {
    let data = Path::new("../data");
    let mut files = String::new();
    for (dir, alias) in [
        ("icons/hicolor/symbolic/actions", "icons/scalable/actions"),
        ("backgrounds", "backgrounds"),
    ] {
        println!("cargo:rerun-if-changed={}", data.join(dir).display());
        for entry in fs::read_dir(data.join(dir))? {
            let name = entry?.file_name().into_string().unwrap();
            files += &format!(
                "<file alias=\"{alias}/{name}\" preprocess=\"xml-stripblanks\">{dir}/{name}</file>\n"
            );
        }
    }
    let out = env::var("OUT_DIR").unwrap();
    let xml = Path::new(&out).join("wassup.gresource.xml");
    fs::write(
        &xml,
        format!(
            "<gresources><gresource prefix=\"/io/github/MawCeron/Wassup\">\n{files}</gresource></gresources>\n"
        ),
    )?;
    let status = Command::new("glib-compile-resources")
        .arg("--sourcedir")
        .arg(data)
        .arg("--target")
        .arg(Path::new(&out).join("wassup.gresource"))
        .arg(&xml)
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other("glib-compile-resources failed"));
    }
    Ok(())
}
