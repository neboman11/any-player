use slint::ComponentHandle;
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let _runtime_guard = runtime.enter();
    if !std::env::args().any(|arg| arg == "--demo") {
        return Err("Production services are not wired yet; use --demo for the offline UI".into());
    }
    let services = Arc::new(any_player::fake::DemoServices::new());
    let progression = services.start_progression();
    let window = any_player::MainWindow::new()?;
    any_player::attach(&window, services.ports());
    window.run()?;
    progression.abort();
    Ok(())
}
