mod app;
mod audio;
mod audio_ui;
mod hypr;
mod luaio;
mod model;
mod settings;
mod update;

fn main() -> iced::Result {
    app::run()
}
