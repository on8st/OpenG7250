include!("../common/build-rev.rs");
fn main() {
    let script = if std::env::var("CARGO_FEATURE_SLOT_B").is_ok() { "memory-slot-b.x" } else { "memory-slot-a.x" };
    link_memory_x(script);
}
