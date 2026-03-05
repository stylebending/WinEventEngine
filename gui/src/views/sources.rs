use iced::widget::{button, column, container, row, text, toggler, Space};
use iced::{Element, Length};

use crate::app::{Message, NotificationType, WinEventApp};

pub fn view(app: &WinEventApp) -> Element<Message> {
    let sources_content = if app.sources.is_empty() {
        column![
            text("No event sources configured").size(16),
            text("Add a source to start monitoring events").size(14),
        ]
        .spacing(10)
    } else {
        let mut sources_list = column![];

        for source in &app.sources {
            let name = source
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("Unnamed");
            let enabled = source
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let source_type = source
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            let source_row = row![
                text(name).size(16).width(Length::Fill),
                text(format!("({})", source_type)).size(14),
                Space::with_width(Length::Fixed(10.0)),
                toggler(enabled)
                    .on_toggle(move |value| Message::ToggleSource(name.to_string(), value)),
                Space::with_width(Length::Fixed(10.0)),
                button("Delete").on_press(Message::DeleteSource(name.to_string())),
            ]
            .spacing(10)
            .padding(10);

            sources_list = sources_list.push(source_row);
        }

        sources_list.spacing(5)
    };

    column![
        // Header
        row![
            text("Event Sources").size(28),
            Space::with_width(Length::Fill),
            button("Refresh").on_press(Message::RefreshSources),
            Space::with_width(Length::Fixed(10.0)),
            button("+ Add Source"),
        ]
        .spacing(20)
        .padding(20),
        // Sources list
        container(sources_content).padding(20).height(Length::Fill),
    ]
    .into()
}
