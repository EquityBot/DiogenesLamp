//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use lamp::pyjson::{self, Map, Value};

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn fixture(name: &str) -> String {
    std::fs::read_to_string(root().join("tests").join("fixtures").join(name)).expect("fixture")
}

pub fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

pub fn write(path: &Path, content: impl AsRef<[u8]>) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("mkdir");
    }
    std::fs::write(path, content).expect("write");
}

pub fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read")
}

pub fn json(text: &str) -> Value {
    pyjson::loads(text).expect("json")
}

pub fn read_json(path: &Path) -> Value {
    json(&read(path))
}

pub fn obj(value: &Value) -> &Map {
    value.as_object().expect("object")
}

/// Run the lamp binary with `args`; stdout and stderr as text.
pub fn lamp(args: &[&str], cwd: Option<&Path>) -> (i32, String, String) {
    let output = lamp_output(args, cwd);
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n"),
    )
}

pub fn lamp_output(args: &[&str], cwd: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lamp"));
    command.args(args).env("COLUMNS", "80");
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command.output().expect("run lamp")
}

/// Encode a solid or patterned image, as the Python tests did with Pillow.
pub fn png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_pixel(width, height, image::Rgb(rgb));
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .expect("png");
    out
}

pub fn encode(image: &image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
    let mut out = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut out), format)
        .expect("encode");
    out
}

/// A JPEG carrying an EXIF orientation tag (APP1 inserted after SOI).
pub fn jpeg_with_orientation(width: u32, height: u32, orientation: u16) -> Vec<u8> {
    let image = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        width,
        height,
        image::Rgb([200, 20, 20]),
    ));
    let jpeg = encode(&image, image::ImageFormat::Jpeg);
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"MM\x00\x2a\x00\x00\x00\x08");
    tiff.extend_from_slice(&1u16.to_be_bytes());
    tiff.extend_from_slice(&0x0112u16.to_be_bytes());
    tiff.extend_from_slice(&3u16.to_be_bytes());
    tiff.extend_from_slice(&1u32.to_be_bytes());
    tiff.extend_from_slice(&orientation.to_be_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_be_bytes());
    let mut app1 = b"Exif\x00\x00".to_vec();
    app1.extend_from_slice(&tiff);
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xff, 0xe1]);
    out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpeg[2..]);
    out
}

pub fn node() -> Option<String> {
    let name = if cfg!(windows) { "node.exe" } else { "node" };
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(name))
            .find(|p| p.is_file())
            .map(|p| p.to_string_lossy().into_owned())
    })
}

/// Run a node script in the repository root with `input` on stdin.
pub fn run_node(script: &str, input: &str) -> String {
    use std::io::Write;
    let node = node().expect("Node-backed parity coverage must run: install node");
    let mut child = Command::new(node)
        .args(["-e", script])
        .current_dir(root())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn node");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("node");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}
