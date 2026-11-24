mod app;
mod audio;
mod audio_ui;
mod diff;
mod docs;
mod hypr;
mod inspect;
mod luaio;
mod model;
mod settings;
mod update;

fn main() -> iced::Result {
    app::run()
}
