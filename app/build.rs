fn main() {
    slint_build::compile_with_config(
        "ui/main.slint",
        slint_build::CompilerConfiguration::new().with_debug_info(true),
    )
    .expect("compile Slint UI");
}
