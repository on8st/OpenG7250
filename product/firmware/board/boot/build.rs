include!("../common/build-rev.rs");
fn main() {
    link_memory_x("memory.x", g7250_layout::BOOT_STAGE);
}
