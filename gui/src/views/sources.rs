use iced::widget::{button, column, container, row, text, Space};
use iced::{Element, Length};

use crate::app::{Message, WinEventApp};

pub fn view(_app: &WinEventApp) -> Element<Message> {
    let sources_content = column![
        text("Event sources (plugins) configuration"),
        text("File Watchers, Window Watchers, Process Monitors, etc."),
        text("This view will list all configured event sources."),
    ]
    .spacing(10);

    column![
        // Header
        row![
            text("Event Sources").size(28),
            Space::with_width(Length::Fill),
            button("+ Add Source").on_press(Message::NavigateTo(crate::app::View::Sources)),
        ]
        .spacing(20)
        .padding(20),
        // Sources list placeholder
        container(sources_content).padding(20).height(Length::Fill),
    ]
    .into()
}
