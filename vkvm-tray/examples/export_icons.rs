//! writes raw RGBA icon files: <out>/<name>-<size>.rgba  (converted to PNG by scripts/dev/rgba2png.py)
fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    for (name, on, hc) in [
        ("vkvm-on-template", true, false),
        ("vkvm-off-template", false, false),
        ("vkvm-on-highcontrast", true, true),
        ("vkvm-off-highcontrast", false, true),
    ] {
        for size in [32u32, 44, 128] {
            let path = format!("{out}/{name}-{size}.rgba");
            std::fs::write(&path, vkvm_tray::icon_rgba(on, size, hc)).expect("write");
            println!("{path}");
        }
    }
}
