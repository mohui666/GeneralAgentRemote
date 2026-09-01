#[cfg(target_arch = "wasm32")]
mod ui;

pub const fn app_name() -> &'static str {
    "Agent Remote Messenger"
}

#[cfg(target_arch = "wasm32")]
pub fn run() {
    yew::Renderer::<ui::App>::new().render();
}
