fn main() {
    for d in vkvm_displays::list() {
        println!(
            "{:<28} id={:<14} x={:<6} y={:<6} {}x{} scale={} primary={}",
            d.name, d.id, d.x, d.y, d.width, d.height, d.scale, d.primary
        );
    }
}
