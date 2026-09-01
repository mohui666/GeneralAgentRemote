#[cfg(target_arch = "wasm32")]
fn main() {
    agent_remote_web::run();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    println!("agent-remote-web is a WebAssembly application");
}
