fn main() {
    let pixels = vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]; // RGBA
    let nq = color_quant::NeuQuant::new(10, 256, &pixels);
    let map_rgb = nq.color_map_rgb();
    let map_rgba = nq.color_map_rgba();
    let idx = nq.index_of(&[255, 0, 0, 255]);
    println!("map_rgb len: {}, map_rgba len: {}, idx: {}", map_rgb.len(), map_rgba.len(), idx);
}
