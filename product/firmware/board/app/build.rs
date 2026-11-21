include!("../common/build-rev.rs");
fn main() {
    let b = std::env::var("CARGO_FEATURE_SLOT_B").is_ok();
    let (script, slot) = if b { ("memory-slot-b.x", g7250_layout::SLOT_B) } else { ("memory-slot-a.x", g7250_layout::SLOT_A) };
    link_memory_x(script, slot);
}
